//! nekoya-stt CLI。
//!
//! P0：配置/状态/协议/状态机（纯逻辑 + 单测）+ `doctor`。
//! P1：`transcribe`（音频 → 千问实时转写，终端打印）。
//! 后续：`run`（常驻守护进程）、`sessions`/`use`（OpenCode HTTP）。

use anyhow::Result;
use clap::{Parser, Subcommand};
use nekoya_stt::{asr, audio, config, config::Config, control, daemon, opencode, state::State};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "nekoya-stt", version, about = "语音识别 → 剪贴板 / VRChat OSC / OpenCode / WayVR 字幕面板")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 常驻守护进程（P2）
    Run,
    /// 环境与连通性自检
    Doctor,
    /// WayVR IPC 自检：连上并发送一条面板文字 + ensure-on（离线，用 mock 服务验证）
    WayvrSelftest {
        /// 要写入 transcript 元素的文字
        #[arg(default_value = "nekoya selftest")]
        text: String,
    },
    /// 音频 → 千问实时转写（P1，终端打印）
    Transcribe {
        /// 回放原始 PCM16/16k/mono 文件（否则用 pw-record 采集麦克风）
        #[arg(long)]
        file: Option<PathBuf>,
        /// 使用服务端 VAD 自动断句（默认手动 commit）
        #[arg(long)]
        auto: bool,
        /// 离线替身：不联网、不采集，直接输出固定事件（验证管线）
        #[arg(long)]
        fake: bool,
    },
    /// 列出 OpenCode 会话（需要服务在运行）
    Sessions,
    /// OpenCode 自检：列根会话；可选向某会话发送一条文本
    OpencodeSelftest {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        send: Option<String>,
    },
    /// 向运行中的 daemon 控制通道发送命令（例如 `ctl act push`、`ctl watch-down`）
    Ctl {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 选择 OpenCode 会话（持久化到 state.toml）
    Use {
        /// 会话 id（`ses…`）
        id: String,
    },
    /// 打印当前配置/状态
    Status,
}

fn main() -> Result<()> {
    // rustls 0.23：任何 WSS 握手前必须先安装默认 CryptoProvider。
    nekoya_stt::tls::install_crypto_provider();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run => {
            let cfg = Config::load()?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(daemon::run(cfg))
        }
        Cmd::WayvrSelftest { text } => wayvr_selftest(text),
        Cmd::Doctor => doctor(),
        Cmd::Transcribe { file, auto, fake } => transcribe(file, auto, fake),
        Cmd::Sessions => sessions(),
        Cmd::OpencodeSelftest { session, send } => opencode_selftest(session, send),
        Cmd::Ctl { args } => ctl(args),
        Cmd::Use { id } => select_session(&id),
        Cmd::Status => status(),
    }
}

fn print_event(ev: &asr::AsrEvent) {
    match ev {
        asr::AsrEvent::SessionReady => println!("[会话] 已就绪"),
        asr::AsrEvent::Preview(t) => println!("[预览] {t}"),
        asr::AsrEvent::Final(t) => println!("[终稿] {t}"),
        asr::AsrEvent::Error(e) => eprintln!("[错误] {e}"),
        asr::AsrEvent::Finished => println!("[会话] 结束"),
        asr::AsrEvent::Other => {}
    }
}

fn wayvr_selftest(text: String) -> Result<()> {
    use nekoya_stt::wayvr_ipc::conn::WayvrLink;
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let mut link = WayvrLink::new(true, "nekoya-selftest");
        if !link.ensure().await {
            anyhow::bail!("无法连接 WayVR IPC：{}", link.last_error().unwrap_or("unknown"));
        }
        link.send(&nekoya_stt::hud::set_text(nekoya_stt::hud::EL_TRANSCRIPT, &text)).await;
        link.send(&nekoya_stt::hud::set_panel_visible(true)).await;
        println!("已发送: SetText(transcript)=\"{text}\" + EnsureVisible");
        Ok(())
    })
}

fn load_corpus(cfg: &Config) -> Option<String> {
    if !cfg.corpus.enabled {
        return None;
    }
    let text = std::fs::read_to_string(config::lexicon_file()).ok()?;
    let joined = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn transcribe(file: Option<PathBuf>, auto: bool, fake: bool) -> Result<()> {
    let cfg = Config::load()?;
    let mode = if auto { asr::Mode::Vad } else { asr::Mode::Manual };
    let from_file = file.is_some();

    let source: Option<Box<dyn audio::AudioSource + Send>> = if fake {
        None
    } else {
        Some(match &file {
            Some(p) => Box::new(audio::FileSource::open(p)?),
            None => Box::new(audio::PwRecordSource::spawn(cfg.audio.source.as_deref())?),
        })
    };

    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let (evt_tx, mut evt_rx) = tokio::sync::mpsc::unbounded_channel::<asr::AsrEvent>();
    let stop = Arc::new(AtomicBool::new(false));

    // 采集线程：按 100ms 节拍把 PCM 块送进通道（文件回放按时序，麦克风实时）。
    if let Some(source) = source {
        let stop = stop.clone();
        let chunk = audio::CHUNK_BYTES;
        thread::spawn(move || {
            let mut source = source;
            let mut buf = vec![0u8; chunk];
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match source.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if audio_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        if from_file {
                            thread::sleep(Duration::from_millis(audio::CHUNK_MS));
                        }
                    }
                    Err(_) => break,
                }
            }
            // audio_tx 析构 → 关闭通道，识别端据此收尾
        });
    }

    // 麦克风模式：回车停止。
    if !from_file && !fake {
        let stop = stop.clone();
        thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            stop.store(true, Ordering::Relaxed);
        });
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        // Ctrl+C 也可停止采集。
        {
            let stop = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                stop.store(true, Ordering::Relaxed);
            });
        }

        if fake {
            for ev in asr::fake_events("你好，这是一次离线冒烟测试。") {
                if evt_tx.send(ev).is_err() {
                    break;
                }
            }
            drop(evt_tx);
            while let Some(ev) = evt_rx.recv().await {
                print_event(&ev);
            }
            return Ok(());
        }

        let key = Config::api_key().unwrap_or_default();
        if key.is_empty() {
            anyhow::bail!("缺少 DASHSCOPE_API_KEY（注入环境变量后重试）");
        }
        let acfg = asr::AsrConfig {
            endpoint: cfg.qwen.endpoint.clone(),
            model: cfg.qwen.model.clone(),
            api_key: key,
            language: cfg.qwen.language.clone(),
            corpus: load_corpus(&cfg),
            vad_silence_ms: 800,
            vad_threshold: 0.2,
        };
        let run = tokio::spawn(async move { asr::transcribe(acfg, mode, audio_rx, evt_tx).await });
        while let Some(ev) = evt_rx.recv().await {
            print_event(&ev);
        }
        run.await??;
        Ok(())
    })
}

fn doctor() -> Result<()> {
    println!("== nekoya-stt doctor ==");

    let cfg_path = config::config_file();
    println!(
        "配置文件    : {} ({})",
        cfg_path.display(),
        if cfg_path.is_file() { "存在" } else { "缺失，用默认值" }
    );
    let cfg = Config::load()?;
    println!("模型        : {}", cfg.qwen.model);
    println!("端点        : {}", cfg.qwen.endpoint);
    println!("OSC 端口    : {}", cfg.osc.port);
    println!("WayVR 联动  : {}", if cfg.wayvr.enabled { "启用" } else { "禁用（默认）" });
    println!("ITN         : {}", if cfg.itn { "开" } else { "关（默认）" });

    match Config::api_key() {
        Some(_) => println!("DASHSCOPE_API_KEY : 已设置"),
        None => println!("DASHSCOPE_API_KEY : **缺失**（运行时无法识别）"),
    }

    let lex = config::lexicon_file();
    println!(
        "词库        : {} ({})",
        lex.display(),
        if lex.is_file() { "存在" } else { "缺失" }
    );

    match probe_wayvr() {
        Ok(true) => println!("WayVR IPC   : 可连接"),
        Ok(false) => println!("WayVR IPC   : 未运行（可接受，默认禁用联动）"),
        Err(e) => println!("WayVR IPC   : 探测失败: {e}"),
    }

    match opencode::discover(&cfg) {
        Some(ep) => {
            println!("OpenCode    : {}（鉴权 {}）", ep.url, if ep.password.is_some() { "有" } else { "无" });
            println!("              可达: {}", if tcp_reachable(&ep.url) { "是" } else { "否" });
        }
        None => println!("OpenCode    : 未发现端点（服务可能未运行）"),
    }

    Ok(())
}

fn status() -> Result<()> {
    let cfg = Config::load()?;
    let st = State::load()?;
    println!("模型        : {}", cfg.qwen.model);
    println!("WayVR       : {}", if cfg.wayvr.enabled { "启用" } else { "禁用" });
    println!("当前会话    : {}", st.session.as_deref().unwrap_or("（未选择）"));
    println!("密钥        : {}", if Config::api_key().is_some() { "已设置" } else { "缺失" });
    Ok(())
}

fn sessions() -> Result<()> {
    let cfg = Config::load()?;
    match opencode::discover(&cfg) {
        Some(ep) => {
            println!("OpenCode 端点: {}", ep.url);
            println!("HTTP 拉取会话列表将在 P4 实现（需要服务运行 + Basic 鉴权）。");
            Ok(())
        }
        None => anyhow::bail!("未发现 OpenCode 端点（服务可能未运行；本工具不会静默拉起服务）"),
    }
}

fn select_session(id: &str) -> Result<()> {
    if !id.starts_with("ses") {
        anyhow::bail!("会话 id 应以 `ses` 开头，收到: {id}");
    }
    let mut st = State::load()?;
    st.session = Some(id.to_string());
    st.save()?;
    println!("已选择会话: {id}");
    Ok(())
}

// ---- 探测辅助（真实 IO） ----

fn probe_wayvr() -> Result<bool> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixStream};
    let addr = SocketAddr::from_abstract_name(nekoya_stt::wayvr_ipc::SOCKET_NAME.as_bytes())?;
    match UnixStream::connect_addr(&addr) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn opencode_selftest(session: Option<String>, send: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let ep = opencode::discover(&cfg).ok_or_else(|| anyhow::anyhow!("未发现 OpenCode 端点"))?;
    println!("端点: {}", ep.url);

    let list = opencode::list_sessions(&ep, 20, cfg.opencode.directory.as_deref())?;
    println!("根会话 {} 条：", list.len());
    for (i, s) in list.iter().enumerate() {
        println!("  [{i}] {}  {}", s.id, s.title);
    }

    if let (Some(sid), Some(text)) = (session, send) {
        opencode::prompt(&ep, &sid, &text)?;
        println!("已发送 prompt 到 {sid}");
    }
    Ok(())
}

fn ctl(args: Vec<String>) -> Result<()> {
    let cmd = args.join(" ");
    if cmd.trim().is_empty() {
        anyhow::bail!("用法: nekoya-stt ctl <命令…>（见 README 控制通道）");
    }
    let sock = config::control_socket();
    match control::request_sync(&sock, &cmd) {
        Ok(reply) => {
            println!("{reply}");
            Ok(())
        }
        Err(e) => anyhow::bail!(
            "无法连接 daemon（{}）：{e}\n提示：先运行 `nekoya-stt run`",
            sock.display()
        ),
    }
}

fn tcp_reachable(url: &str) -> bool {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or(url);
    let hostport = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(80)),
        None => (hostport.to_string(), 80),
    };
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else { return false };
    match addrs.next() {
        Some(addr) => TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok(),
        None => false,
    }
}

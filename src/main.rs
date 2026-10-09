//! nekoya-stt CLI。
//!
//! P0 范围：配置/状态/协议/状态机（纯逻辑，含单测）+ 环境自检（`doctor`）。
//! 后续阶段：`run`（常驻守护进程）、`sessions`/`use`（OpenCode HTTP）。

use anyhow::Result;
use clap::{Parser, Subcommand};
use nekoya_stt::{config, config::Config, opencode, state::State};

#[derive(Parser)]
#[command(
    name = "nekoya-stt",
    version,
    about = "语音识别 → 剪贴板 / VRChat OSC / OpenCode / WayVR 字幕面板"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 常驻守护进程（后续阶段实现）
    Run,
    /// 环境与连通性自检
    Doctor,
    /// 列出 OpenCode 会话（需要服务在运行）
    Sessions,
    /// 选择 OpenCode 会话（持久化到 state.toml）
    Use {
        /// 会话 id（`ses…`）
        id: String,
    },
    /// 打印当前配置/状态
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run => {
            let cfg = Config::load()?;
            println!("nekoya-stt: run 主体（音频/ASR/控制 socket）将在后续阶段实现。");
            println!("  模型      : {}", cfg.qwen.model);
            println!("  端点      : {}", cfg.qwen.endpoint);
            println!(
                "  WayVR     : {}",
                if cfg.wayvr.enabled {
                    "启用"
                } else {
                    "禁用（默认）"
                }
            );
            anyhow::bail!("run 尚未实现（P0 骨架）");
        }
        Cmd::Doctor => doctor(),
        Cmd::Sessions => sessions(),
        Cmd::Use { id } => select_session(&id),
        Cmd::Status => status(),
    }
}

fn doctor() -> Result<()> {
    println!("== nekoya-stt doctor ==");

    let cfg_path = config::config_file();
    println!(
        "配置文件    : {} ({})",
        cfg_path.display(),
        if cfg_path.is_file() {
            "存在"
        } else {
            "缺失，用默认值"
        }
    );
    let cfg = Config::load()?;
    println!("模型        : {}", cfg.qwen.model);
    println!("端点        : {}", cfg.qwen.endpoint);
    println!("OSC 端口    : {}", cfg.osc.port);
    println!(
        "WayVR 联动  : {}",
        if cfg.wayvr.enabled {
            "启用"
        } else {
            "禁用（默认）"
        }
    );
    println!(
        "ITN         : {}",
        if cfg.itn { "开" } else { "关（默认）" }
    );

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

    // WayVR IPC（抽象 socket）
    match probe_wayvr() {
        Ok(true) => println!("WayVR IPC   : 可连接"),
        Ok(false) => println!("WayVR IPC   : 未运行（可接受，默认禁用联动）"),
        Err(e) => println!("WayVR IPC   : 探测失败: {e}"),
    }

    // OpenCode 发现
    match discover_opencode(&cfg) {
        Some(ep) => {
            println!(
                "OpenCode    : {}（鉴权 {}）",
                ep.url,
                if ep.password.is_some() { "有" } else { "无" }
            );
            println!(
                "              可达: {}",
                if tcp_reachable(&ep.url) { "是" } else { "否" }
            );
        }
        None => println!("OpenCode    : 未发现端点（服务可能未运行）"),
    }

    Ok(())
}

fn status() -> Result<()> {
    let cfg = Config::load()?;
    let st = State::load()?;
    println!("模型        : {}", cfg.qwen.model);
    println!(
        "WayVR       : {}",
        if cfg.wayvr.enabled {
            "启用"
        } else {
            "禁用"
        }
    );
    println!(
        "当前会话    : {}",
        st.session.as_deref().unwrap_or("（未选择）")
    );
    println!(
        "密钥        : {}",
        if Config::api_key().is_some() {
            "已设置"
        } else {
            "缺失"
        }
    );
    Ok(())
}

fn sessions() -> Result<()> {
    let cfg = Config::load()?;
    match discover_opencode(&cfg) {
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

fn opencode_state_file() -> std::path::PathBuf {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local/state"))
                .unwrap_or_default()
        });
    state_home.join("opencode/service.json")
}

fn discover_opencode(cfg: &Config) -> Option<opencode::Endpoint> {
    let reg = std::fs::read_to_string(opencode_state_file()).ok();
    let settings =
        std::fs::read_to_string(config::xdg_config_home().join("opencode/service.json")).ok();
    opencode::resolve(
        cfg.opencode.url.as_deref(),
        cfg.opencode.password.as_deref(),
        &cfg.opencode.username,
        reg.as_deref(),
        settings.as_deref(),
    )
}

fn tcp_reachable(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let hostport = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(80)),
        None => (hostport.to_string(), 80),
    };
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };
    match addrs.next() {
        Some(addr) => TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok(),
        None => false,
    }
}

//! 常驻守护进程：采集 → 千问 ASR → 引擎 → HUD/动作；本地控制 socket。
//!
//! 行为契约（P2）：
//! - 手表键（`watch-down`/`watch-up`）：按下即开始录制；按住 ≥ `ptt.start_after_ms`
//!   才开会话（此前音频在通道里缓冲、不浪费连接）；松手短于阈值 → 丢弃并显示主面板；
//!   松手 ≥ 阈值 → 收尾并把终稿推送到 OpenCode。
//! - 主面板 PTT（`ptt-down`/`ptt-up`）：按下立即开会话；松手收尾，仅更新字幕。
//! - WayVR 默认禁用；连不上降级，不影响识别。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{mpsc, Mutex, Notify};

use crate::asr::{self, AsrConfig, AsrEvent, Mode};
use crate::audio::{self, PwRecordSource};
use crate::config::{self, Config};
use crate::control::{self, Cmd};
use crate::engine::Engine;
use crate::hud;
use crate::opencode::{self, SessionInfo};
use crate::sinks;
use crate::state::State as PersistedState;
use crate::wayvr_ipc::conn::WayvrLink;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Watch,
    Panel,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Watch => "手表",
            Kind::Panel => "面板",
        }
    }
}

struct Active {
    kind: Kind,
    stop: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

struct Inner {
    engine: Engine,
    session: Option<String>,
    session_title: Option<String>,
    active: Option<Active>,
    link: WayvrLink,
    last_status: String,
    /// 最近一次拉取的会话列表（用于 `act select N`）。
    sessions: Vec<SessionInfo>,
    list_visible: bool,
}

#[derive(Clone)]
pub struct Daemon {
    inner: Arc<Mutex<Inner>>,
    cfg: Config,
    corpus: Option<String>,
}

impl Daemon {
    pub fn new(cfg: Config, corpus: Option<String>) -> Self {
        let link = WayvrLink::new(cfg.wayvr.enabled, cfg.wayvr.client_name.clone());
        let (session, session_title) = match PersistedState::load() {
            Ok(s) => (s.session, s.title),
            Err(_) => (None, None),
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                engine: Engine::new(),
                session,
                session_title,
                active: None,
                link,
                last_status: "空闲".into(),
                sessions: Vec::new(),
                list_visible: false,
            })),
            cfg,
            corpus,
        }
    }

    fn asr_config(&self) -> AsrConfig {
        AsrConfig {
            endpoint: self.cfg.qwen.endpoint.clone(),
            model: self.cfg.qwen.model.clone(),
            api_key: Config::api_key().unwrap_or_default(),
            language: self.cfg.qwen.language.clone(),
            corpus: self.corpus.clone(),
            vad_silence_ms: 800,
            vad_threshold: 0.2,
        }
    }

    /// HUD：写转写文字。
    async fn hud_transcript(&self, text: &str) {
        let packet = hud::set_text(hud::EL_TRANSCRIPT, text);
        let mut inner = self.inner.lock().await;
        inner.link.send(&packet).await;
    }

    /// HUD：写状态行 + 更新本地状态。
    async fn hud_status(&self, text: &str) {
        {
            let mut inner = self.inner.lock().await;
            inner.last_status = text.to_string();
        }
        let packet = hud::set_text(hud::EL_STATUS, text);
        let mut inner = self.inner.lock().await;
        inner.link.send(&packet).await;
    }

    async fn hud_session(&self) {
        let (title, has_session) = {
            let inner = self.inner.lock().await;
            (inner.session_title.clone(), inner.session.is_some())
        };
        let text = match title {
            Some(t) if !t.trim().is_empty() => format!("会话：{t}"),
            _ if has_session => "会话：（已选择）".to_string(),
            _ => "会话：（未选择）".to_string(),
        };
        let packet = hud::set_text(hud::EL_SESSION, &text);
        let mut inner = self.inner.lock().await;
        inner.link.send(&packet).await;
    }

    async fn link_add_init(&self) {
        // 仅当启用且已连接时，补写一次会话名与状态。
        let connected = {
            let mut inner = self.inner.lock().await;
            inner.link.ensure().await
        };
        if connected {
            self.hud_session().await;
            let status = {
                let inner = self.inner.lock().await;
                inner.last_status.clone()
            };
            self.hud_status(&status).await;
        }
    }

    pub async fn handle(&self, cmd: Cmd) -> String {
        match cmd {
            Cmd::WatchDown => self.start(Kind::Watch).await,
            Cmd::PttDown => self.start(Kind::Panel).await,
            Cmd::WatchUp => self.stop(Kind::Watch).await,
            Cmd::PttUp => self.stop(Kind::Panel).await,
            Cmd::Show => {
                let packet = hud::set_panel_visible(true);
                let mut inner = self.inner.lock().await;
                inner.link.send(&packet).await;
                "ok".into()
            }
            Cmd::Status => {
                let inner = self.inner.lock().await;
                let phase = inner.engine.phase().label();
                let last = inner.engine.last_final().unwrap_or("（无）");
                let session = inner.session.as_deref().unwrap_or("（未选择）");
                let link = if inner.link.is_connected() { "已连接" } else { "未连接" };
                format!("阶段={phase} 会话={session} WayVR={link} 最近终稿={last}")
            }
            Cmd::ActCopy => {
                let text = {
                    let inner = self.inner.lock().await;
                    inner.engine.last_final().map(str::to_string)
                };
                match text {
                    Some(t) => match tokio::task::spawn_blocking(move || sinks::wl_copy(&t)).await {
                        Ok(Ok(())) => {
                            self.hud_status("已复制到剪贴板（宿主）").await;
                            "ok".into()
                        }
                        Ok(Err(e)) => {
                            self.hud_status(&format!("复制失败：{e}")).await;
                            format!("err: {e}")
                        }
                        Err(e) => format!("err: {e}"),
                    },
                    None => {
                        self.hud_status("无可用转写").await;
                        "empty".into()
                    }
                }
            }
            Cmd::ActOsc => {
                let text = {
                    let inner = self.inner.lock().await;
                    inner.engine.last_final().map(str::to_string)
                };
                match text {
                    Some(t) => {
                        let capped = crate::actions::truncate_chars(&t, self.cfg.osc.chatbox_max_chars);
                        let port = self.cfg.osc.port;
                        match tokio::task::spawn_blocking(move || sinks::send_osc(port, &capped)).await {
                            Ok(Ok(())) => {
                                self.hud_status("已发送 VRChat OSC").await;
                                "ok".into()
                            }
                            Ok(Err(e)) => {
                                self.hud_status(&format!("OSC 失败：{e}")).await;
                                format!("err: {e}")
                            }
                            Err(e) => format!("err: {e}"),
                        }
                    }
                    None => {
                        self.hud_status("无可用转写").await;
                        "empty".into()
                    }
                }
            }
            Cmd::ActPush => self.push_opencode().await,
            Cmd::ActList => self.toggle_session_list().await,
            Cmd::ActSelect(n) => self.select_session_index(n).await,
        }
    }

    async fn push_opencode(&self) -> String {
        let (text, session) = {
            let inner = self.inner.lock().await;
            (inner.engine.last_final().map(str::to_string), inner.session.clone())
        };
        let (text, session) = match (text, session) {
            (Some(t), Some(s)) => (t, s),
            (_, None) => {
                self.hud_status("未选择会话").await;
                return "no-session".into();
            }
            (None, _) => {
                self.hud_status("无可用转写").await;
                return "empty".into();
            }
        };

        let cfg = self.cfg.clone();
        let res = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let ep = opencode::discover(&cfg).ok_or_else(|| "未发现 OpenCode 端点".to_string())?;
            // 取会话当前 agent，让推送跟随 TUI 的选择（拿不到则用默认）。
            let agent = opencode::get_session_agent(&ep, &session);
            opencode::prompt(&ep, &session, &text, agent.as_deref()).map_err(|e| e.to_string())
        })
        .await;

        match res {
            Ok(Ok(())) => {
                self.hud_status("已推送到 OpenCode").await;
                "ok".into()
            }
            Ok(Err(e)) => {
                self.hud_status(&format!("推送失败：{e}")).await;
                format!("err: {e}")
            }
            Err(e) => {
                self.hud_status(&format!("推送失败：{e}")).await;
                format!("err: {e}")
            }
        }
    }

    /// `act list`：切换会话列表；打开时拉取并填充 64 行槽。
    async fn toggle_session_list(&self) -> String {
        let visible = {
            let inner = self.inner.lock().await;
            inner.list_visible
        };
        if visible {
            let mut inner = self.inner.lock().await;
            inner.link.send(&hud::set_visible(hud::EL_LIST, false)).await;
            inner.list_visible = false;
            drop(inner);
            self.hud_status("已关闭会话列表").await;
            return "hidden".into();
        }

        let cfg = self.cfg.clone();
        let fetched = tokio::task::spawn_blocking(move || -> Result<Vec<SessionInfo>, String> {
            let ep = opencode::discover(&cfg).ok_or_else(|| "未发现 OpenCode 端点".to_string())?;
            opencode::list_sessions(&ep, hud::LIST_ROWS as u32, cfg.opencode.directory.as_deref())
                .map_err(|e| e.to_string())
        })
        .await;

        let list = match fetched {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => {
                self.hud_status(&format!("会话列表失败：{e}")).await;
                return format!("err: {e}");
            }
            Err(e) => {
                self.hud_status(&format!("会话列表失败：{e}")).await;
                return format!("err: {e}");
            }
        };

        let n = list.len().min(hud::LIST_ROWS);
        let link_ok;
        {
            let mut inner = self.inner.lock().await;
            // 行按钮默认可见（XML 里没有 display="none"），因此每轮都要显式收口：
            // 用到的行写标题并显示；未使用的行清空并隐藏。
            for i in 0..hud::LIST_ROWS {
                match list.get(i) {
                    Some(s) => {
                        inner
                            .link
                            .send(&hud::set_text(&hud::list_row_text_id(i), &s.row_label()))
                            .await;
                        inner
                            .link
                            .send(&hud::set_visible(&hud::list_row_id(i), true))
                            .await;
                    }
                    None => {
                        inner.link.send(&hud::set_text(&hud::list_row_text_id(i), "")).await;
                        inner.link.send(&hud::set_visible(&hud::list_row_id(i), false)).await;
                    }
                }
            }
            inner.link.send(&hud::set_visible(hud::EL_LIST, true)).await;
            link_ok = inner.link.is_connected();
            inner.sessions = list;
            inner.list_visible = true;
        }
        self.hud_status(&format!("会话列表（{n}）")).await;
        format!("ok: {n} link={}", if link_ok { 1 } else { 0 })
    }

    /// `act select N`：选择列表第 N 项并持久化。
    async fn select_session_index(&self, n: usize) -> String {
        let (id, title) = {
            let inner = self.inner.lock().await;
            match inner.sessions.get(n) {
                Some(s) => (s.id.clone(), s.title.clone()),
                None => return "out-of-range".into(),
            }
        };
        if let Ok(mut st) = PersistedState::load() {
            st.session = Some(id.clone());
            st.title = Some(title.clone());
            let _ = st.save();
        }
        {
            let mut inner = self.inner.lock().await;
            inner.session = Some(id.clone());
            inner.session_title = Some(title.clone());
            inner.link.send(&hud::set_visible(hud::EL_LIST, false)).await;
            inner.list_visible = false;
        }
        self.hud_session().await;
        self.hud_status(&format!("已选择会话：{title}")).await;
        "ok".into()
    }

    async fn start(&self, kind: Kind) -> String {
        let mut inner = self.inner.lock().await;
        if inner.active.is_some() {
            return "busy".into();
        }
        let start_res = match kind {
            Kind::Watch => inner.engine.start_watch(),
            Kind::Panel => inner.engine.start_ptt(),
        };
        if start_res.is_err() {
            return "busy".into();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        inner.active = Some(Active { kind, stop: stop.clone(), notify: notify.clone() });
        drop(inner);

        let daemon = self.clone();
        let src = self.cfg.audio.source.clone();
        let start_after = Duration::from_millis(self.cfg.ptt.start_after_ms);
        let acfg = self.asr_config();
        tokio::spawn(async move {
            daemon.run_recording(kind, src, start_after, stop, notify, acfg).await;
        });

        "ok".into()
    }

    async fn stop(&self, kind: Kind) -> String {
        let inner = self.inner.lock().await;
        match inner.active.as_ref() {
            Some(a) if a.kind == kind => {
                a.stop.store(true, Ordering::Relaxed);
                a.notify.notify_waiters();
                // active 由 run_recording 结束时清除；这里不立刻清，避免竞态
                "ok".into()
            }
            Some(_) => "busy-other".into(),
            None => "noop".into(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_recording(
        &self,
        kind: Kind,
        source: Option<String>,
        start_after: Duration,
        stop: Arc<AtomicBool>,
        notify: Arc<Notify>,
        acfg: AsrConfig,
    ) {
        self.hud_status(&format!("{}：按住中…", kind.label())).await;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(512);
        let src = match PwRecordSource::spawn(source.as_deref()) {
            Ok(s) => s,
            Err(e) => {
                self.hud_status(&format!("采集失败：{e}")).await;
                self.finish_recording(kind).await;
                return;
            }
        };
        spawn_capture(src, stop.clone(), tx);

        // 安全阀：55s 强制收尾。
        {
            let stop = stop.clone();
            let notify = notify.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(55)).await;
                stop.store(true, Ordering::Relaxed);
                notify.notify_waiters();
            });
        }

        // 手表键：延迟开流；早松手 → 丢弃 + 显示主面板。
        if kind == Kind::Watch {
            tokio::select! {
                _ = tokio::time::sleep(start_after) => {}
                _ = notify.notified() => {
                    stop.store(true, Ordering::Relaxed);
                    drop(rx); // 关闭通道，采集线程结束
                    self.hud_status("短按：已打开面板").await;
                    let packet = hud::set_panel_visible(true);
                    {
                        let mut inner = self.inner.lock().await;
                        inner.link.send(&packet).await;
                    }
                    self.finish_recording(kind).await;
                    return;
                }
            }
            // Armed → Streaming
            {
                let mut inner = self.inner.lock().await;
                let _ = inner.engine.begin_stream();
            }
            self.hud_status("聆听中…").await;
        } else {
            self.hud_status("聆听中…").await;
        }

        let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AsrEvent>();
        let task = tokio::spawn(async move { asr::transcribe(acfg, Mode::Manual, rx, evt_tx).await });

        let mut got_final: Option<String> = None;
        let mut last_preview = String::new();
        while let Some(ev) = evt_rx.recv().await {
            match ev {
                AsrEvent::SessionReady => {}
                AsrEvent::Preview(t) => {
                    // 去重：预览常重复同一句，避免无意义的面板刷新。
                    if t != last_preview {
                        last_preview = t.clone();
                        self.hud_transcript(&t).await;
                    }
                }
                AsrEvent::Final(t) => {
                    self.hud_transcript(&t).await;
                    got_final = Some(t);
                }
                AsrEvent::Error(e) => {
                    self.hud_status(&format!("错误：{e}")).await;
                }
                AsrEvent::Finished | AsrEvent::Other => {}
            }
        }
        let _ = task.await;

        {
            let mut inner = self.inner.lock().await;
            inner.engine.finish(got_final.clone());
        }

        match kind {
            Kind::Watch => {
                // 手表长按：只推送 OpenCode，不写剪贴板。
                if got_final.is_some() {
                    let _ = self.push_opencode().await;
                } else {
                    self.hud_status("未识别到语音").await;
                }
            }
            Kind::Panel => {
                match &got_final {
                    Some(t) => {
                        // 主面板 PTT：终稿后自动复制到宿主剪贴板（可配置关闭）。
                        if self.cfg.clipboard.auto_copy {
                            let text = t.clone();
                            let copied =
                                tokio::task::spawn_blocking(move || sinks::wl_copy(&text)).await;
                            match copied {
                                Ok(Ok(())) => self.hud_status("已识别并复制到剪贴板").await,
                                _ => self.hud_status("转写完成（复制失败）").await,
                            }
                        } else {
                            self.hud_status("转写完成").await;
                        }
                    }
                    None => self.hud_status("未识别到语音").await,
                }
            }
        }

        self.finish_recording(kind).await;
    }

    async fn finish_recording(&self, kind: Kind) {
        let mut inner = self.inner.lock().await;
        if inner.active.as_ref().map(|a| a.kind) == Some(kind) {
            inner.active = None;
        }
        // 若已结束且引擎仍在监听态，复位为空闲。
        if inner.engine.phase().is_listening() {
            inner.engine.finish(None);
        }
        if inner.last_status.contains("按住中") || inner.last_status.contains("聆听中") {
            inner.last_status = "空闲".into();
        }
        let status = inner.last_status.clone();
        drop(inner);
        self.hud_status(&status).await;
    }
}

/// 采集线程：读 `pw-record`，按块送进通道；`stop` 置位或通道关闭即退出。
fn spawn_capture(mut src: PwRecordSource, stop: Arc<AtomicBool>, tx: mpsc::Sender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; audio::CHUNK_BYTES];
        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match audio::AudioSource::read(&mut src, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// 运行常驻服务。
pub async fn run(cfg: Config) -> Result<()> {
    let corpus = load_corpus(&cfg);
    let sock: PathBuf = config::control_socket();

    let daemon = Daemon::new(cfg, corpus);
    daemon.link_add_init().await;

    {
        let mut inner = daemon.inner.lock().await;
        let enabled = inner.link.is_enabled();
        let connected = inner.link.is_connected();
        inner.last_status = if enabled && connected {
            "空闲（WayVR 已连接）".into()
        } else if enabled {
            "空闲（WayVR 未连接）".into()
        } else {
            "空闲（WayVR 已禁用）".into()
        };
    }

    // 控制 socket 与后台重连循环。
    let sock_for_reconnect = sock.clone();
    let d = daemon.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut inner = d.inner.lock().await;
            let was = inner.link.is_connected();
            let now = inner.link.ensure().await;
            if now && !was {
                inner.last_status = "空闲（WayVR 已连接）".into();
                let s = inner.last_status.clone();
                drop(inner);
                d.hud_session().await;
                d.hud_status(&s).await;
            }
        }
    });
    let _ = sock_for_reconnect;

    println!("nekoya-stt 已启动。控制 socket: {}", sock.display());
    println!("WayVR 联动: {}", if daemon.inner.lock().await.link.is_enabled() { "启用" } else { "禁用（默认）" });

    control::serve(&sock, {
        let d = daemon.clone();
        move |cmd: Cmd| {
            let d = d.clone();
            async move { d.handle(cmd).await }
        }
    })
    .await?;

    Ok(())
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

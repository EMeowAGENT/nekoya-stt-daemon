//! WayVR IPC 连接（异步）：抽象 socket 连接 + 握手 + 发送（fire-and-forget）。
//!
//! 只使用 `WlxModifyPanel` / `WlxOverlaySetVisible`，绝不触达进程类命令。
//! 默认禁用；连接失败按退避重试，不影响其它功能（降级）。

use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::{
    encode, parse_server_payload, Handshake, PacketClient, ServerEvent, MAX_PACKET, SOCKET_NAME,
};

async fn write_packet(stream: &mut UnixStream, packet: &PacketClient) -> Result<()> {
    let bytes = encode(packet)?;
    stream.write_all(&bytes).await.context("write ipc frame")?;
    Ok(())
}

async fn read_server_event(stream: &mut UnixStream) -> Result<ServerEvent> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.context("read ipc header")?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_PACKET {
        bail!("packet too large ({len} > {MAX_PACKET})");
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.context("read ipc payload")?;
    parse_server_payload(&payload)
}

/// 单次连接：连接 → 握手 → 返回已认证的连接。
pub async fn connect_once(client_name: &str) -> Result<UnixStream> {
    let addr = SocketAddr::from_abstract_name(SOCKET_NAME.as_bytes())
        .context("build abstract socket addr")?;
    // std 阻塞连接抽象名（本机本地 socket，瞬时完成），再转为 tokio 异步流。
    let std_stream = std::os::unix::net::UnixStream::connect_addr(&addr)
        .context("connect wayvr ipc (abstract)")?;
    std_stream.set_nonblocking(true).context("set nonblocking")?;
    let mut stream = UnixStream::from_std(std_stream).context("wrap std unix stream")?;

    write_packet(&mut stream, &PacketClient::Handshake(Handshake::new(client_name)?)).await?;

    match read_server_event(&mut stream).await? {
        ServerEvent::HandshakeSuccess { .. } => Ok(stream),
        ServerEvent::Disconnect { reason } => bail!("wayvr 拒绝连接: {reason}"),
        ServerEvent::Other => bail!("wayvr 首包不是 HandshakeSuccess"),
    }
}

/// 带退避重连的 IPC 链路。`enabled=false` 时全部为空操作。
pub struct WayvrLink {
    enabled: bool,
    client_name: String,
    stream: Option<UnixStream>,
    backoff_until: Option<Instant>,
    backoff: Duration,
    last_error: Option<String>,
}

impl WayvrLink {
    pub fn new(enabled: bool, client_name: impl Into<String>) -> Self {
        let client_name = client_name.into();
        let client_name = if client_name.is_empty() { "nekoya-stt".to_string() } else { client_name };
        Self {
            enabled,
            client_name,
            stream: None,
            backoff_until: None,
            backoff: Duration::from_millis(500),
            last_error: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// 确保连接（若已断开且退避已过则尝试重连）。返回是否已连接。
    pub async fn ensure(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.stream.is_some() {
            return true;
        }
        if let Some(t) = self.backoff_until {
            if Instant::now() < t {
                return false;
            }
        }
        match connect_once(&self.client_name).await {
            Ok(s) => {
                self.stream = Some(s);
                self.backoff = Duration::from_millis(500);
                self.backoff_until = None;
                self.last_error = None;
                true
            }
            Err(e) => {
                self.last_error = Some(e.to_string());
                self.backoff = (self.backoff * 2).min(Duration::from_secs(30));
                self.backoff_until = Some(Instant::now() + self.backoff);
                false
            }
        }
    }

    /// 尽力发送；失败则断开并在下次 `ensure` 重连。
    pub async fn send(&mut self, packet: &PacketClient) {
        if !self.ensure().await {
            return;
        }
        let Some(stream) = self.stream.as_mut() else { return };
        if let Err(e) = write_packet(stream, packet).await {
            self.last_error = Some(e.to_string());
            self.stream = None;
        }
    }
}

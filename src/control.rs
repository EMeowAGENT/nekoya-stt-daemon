//! 本地控制通道（`$XDG_RUNTIME_DIR/nekoya-stt.sock`）。
//!
//! 面板按钮 / 手表键（WayVR 的 `::ShellExec`）与 CLI 都用它把命令发给 daemon，
//! daemon 处理后回一行文本。
//!
//! 说明：`Ctrl+V` / `回车` 由 WayVR 面板自带的 `::SendKey` 完成，**不经过**本通道。

use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// 手表键按下（开始录制，延迟开流）。
    WatchDown,
    /// 手表键松开（短按丢弃 / 长按推送）。
    WatchUp,
    /// 手表键短按取消（丢弃本次录音，不推送；面板切换由 WayVR 的 ::OverlayToggle 负责）。
    WatchCancel,
    /// 主面板 PTT 按下（立即开流）。
    PttDown,
    /// 主面板 PTT 松开（收尾，仅更新字幕）。
    PttUp,
    /// 确保显示主面板。
    Show,
    /// 查询状态。
    Status,
    /// 复制最近终稿到宿主剪贴板。
    ActCopy,
    /// 经 VRChat OSC 发送最近终稿。
    ActOsc,
    /// 推送到 OpenCode 会话。
    ActPush,
    /// 展开会话列表。
    ActList,
    /// 选择会话列表中的第 n 项。
    ActSelect(usize),
}

/// 解析一行命令文本（`act copy` / `watch-down` / `act select 3` …）。
pub fn parse(line: &str) -> Option<Cmd> {
    let mut it = line.split_whitespace();
    match it.next()? {
        "watch-down" => Some(Cmd::WatchDown),
        "watch-up" => Some(Cmd::WatchUp),
        "watch-cancel" => Some(Cmd::WatchCancel),
        "ptt-down" => Some(Cmd::PttDown),
        "ptt-up" => Some(Cmd::PttUp),
        "show" => Some(Cmd::Show),
        "status" => Some(Cmd::Status),
        // 裸动作（面板经 `nekoya-stt act <动作>` 转发时会命中）
        "copy" => Some(Cmd::ActCopy),
        "osc" => Some(Cmd::ActOsc),
        "push" => Some(Cmd::ActPush),
        "list" => Some(Cmd::ActList),
        "select" => it.next()?.parse::<usize>().ok().map(Cmd::ActSelect),
        "act" => match it.next()? {
            "copy" => Some(Cmd::ActCopy),
            "osc" => Some(Cmd::ActOsc),
            "push" => Some(Cmd::ActPush),
            "list" => Some(Cmd::ActList),
            "select" => it.next()?.parse::<usize>().ok().map(Cmd::ActSelect),
            _ => None,
        },
        _ => None,
    }
}

/// 把 CLI 的 `nekoya-stt act <args…>` 映射为控制通道命令行。
///
/// - 生命周期类直接转发：`act watch-down` → `watch-down`
/// - 其余按动作转发：`act copy` → `act copy`、`act select 3` → `act select 3`
pub fn act_to_control(args: &[String]) -> String {
    const LIFECYCLE: [&str; 7] = ["watch-down", "watch-up", "watch-cancel", "ptt-down", "ptt-up", "show", "status"];
    let joined = args.join(" ");
    match args.first().map(String::as_str) {
        Some(first) if LIFECYCLE.contains(&first) => joined,
        _ => format!("act {joined}"),
    }
}

/// 启动服务端；对每条连接读取一行命令，调用 `handler` 返回响应文本。
pub async fn serve<F, Fut>(path: &Path, handler: F) -> std::io::Result<()>
where
    F: Fn(Cmd) -> Fut + Send + Sync + 'static + Clone,
    Fut: std::future::Future<Output = String> + Send,
{
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let handler = handler.clone();
        tokio::spawn(async move {
            let _ = handle_conn(stream, handler).await;
        });
    }
}

async fn handle_conn<F, Fut>(mut stream: UnixStream, handler: F) -> std::io::Result<()>
where
    F: Fn(Cmd) -> Fut,
    Fut: std::future::Future<Output = String>,
{
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.contains(&b'\n') || buf.len() > 4096 {
            break;
        }
    }
    let line = String::from_utf8_lossy(&buf);
    let reply = match parse(line.trim()) {
        Some(cmd) => handler(cmd).await,
        None => "unknown command".to_string(),
    };
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.shutdown().await.ok();
    Ok(())
}

/// 客户端：同步发送一行命令并读取一行响应（供 CLI / 调试）。
pub fn request_sync(path: &Path, cmd: &str) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.write_all(cmd.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut out = String::new();
    stream.read_to_string(&mut out)?;
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_commands() {
        assert_eq!(parse("watch-down"), Some(Cmd::WatchDown));
        assert_eq!(parse("watch-up\n"), Some(Cmd::WatchUp));
        assert_eq!(parse("watch-cancel"), Some(Cmd::WatchCancel));
        assert_eq!(parse("ptt-down"), Some(Cmd::PttDown));
        assert_eq!(parse("act copy"), Some(Cmd::ActCopy));
        assert_eq!(parse("act osc"), Some(Cmd::ActOsc));
        assert_eq!(parse("act push"), Some(Cmd::ActPush));
        assert_eq!(parse("act list"), Some(Cmd::ActList));
        assert_eq!(parse("act select 3"), Some(Cmd::ActSelect(3)));
        assert_eq!(parse("show"), Some(Cmd::Show));
        assert_eq!(parse("status"), Some(Cmd::Status));
        // 裸动作形式
        assert_eq!(parse("copy"), Some(Cmd::ActCopy));
        assert_eq!(parse("select 5"), Some(Cmd::ActSelect(5)));
        assert_eq!(parse("nonsense"), None);
    }

    #[test]
    fn act_subcommand_maps_to_control_line() {
        let v = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(act_to_control(&v("watch-down")), "watch-down");
        assert_eq!(act_to_control(&v("ptt-up")), "ptt-up");
        assert_eq!(act_to_control(&v("show")), "show");
        assert_eq!(act_to_control(&v("copy")), "act copy");
        assert_eq!(act_to_control(&v("push")), "act push");
        assert_eq!(act_to_control(&v("select 3")), "act select 3");
        // 映射结果必须能被解析
        assert_eq!(parse(&act_to_control(&v("watch-down"))), Some(Cmd::WatchDown));
        assert_eq!(parse(&act_to_control(&v("copy"))), Some(Cmd::ActCopy));
        assert_eq!(parse(&act_to_control(&v("select 3"))), Some(Cmd::ActSelect(3)));
    }
}

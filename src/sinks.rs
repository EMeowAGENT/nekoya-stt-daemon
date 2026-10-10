//! 输出 sinks：宿主剪贴板（`wl-copy`）、VRChat OSC（UDP `/chatbox/input`）。
//!
//! 纯编码部分（OSC 报文）可离线单测；`wl_copy` / `send_osc` 为真实 IO。

use anyhow::{bail, Context};

/// VRChat chatbox 文本上限（字符）。
pub const CHATBOX_MAX: usize = 144;

// ---------------- OSC ----------------

/// 把字符串编码为 OSC string：UTF-8 字节 + 至少一个 NUL，补齐到 4 字节边界。
pub fn osc_string(s: &str) -> Vec<u8> {
    let mut out = s.as_bytes().to_vec();
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

/// 构造 `/chatbox/input <text> true` 的 OSC 报文（`true` 用 typetag 'T'，无数据段）。
pub fn osc_chatbox_packet(text: &str) -> Vec<u8> {
    let mut packet = osc_string("/chatbox/input");
    packet.extend_from_slice(&osc_string(",sT"));
    packet.extend_from_slice(&osc_string(text));
    packet
}

/// 经 UDP 发送 chatbox 文本到 VRChat（默认端口 9000）。
pub fn send_osc(port: u16, text: &str) -> anyhow::Result<()> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("127.0.0.1:0").context("bind udp")?;
    sock.connect(("127.0.0.1", port)).context("connect udp")?;
    sock.send(&osc_chatbox_packet(text)).context("send osc")?;
    Ok(())
}

// ---------------- 宿主剪贴板 ----------------

/// 写宿主 Wayland 剪贴板（`wl-copy`）。
///
/// 注意：只写宿主剪贴板；WayVR 内嵌合成器的剪贴板外部无法写入（见 docs/architecture.md）。
pub fn wl_copy(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("无法启动 wl-copy: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .context("写入 wl-copy stdin")?;
    }
    // wl-copy 会 fork 成常驻进程持有 selection；父进程随即返回。
    let status = child.wait().context("等待 wl-copy")?;
    if !status.success() {
        bail!("wl-copy 退出码非零: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_string_padding() {
        assert_eq!(osc_string(""), vec![0, 0, 0, 0]);
        assert_eq!(osc_string("/a"), vec![b'/', b'a', 0, 0]);
        assert_eq!(osc_string("/abc"), vec![b'/', b'a', b'b', b'c', 0, 0, 0, 0]);
        // 4 + 1 -> 8
    }

    #[test]
    fn chatbox_packet_shape() {
        let p = osc_chatbox_packet("hi");
        // address "/chatbox/input"(14) + NUL + pad2 = 16；typetag ",sT" = 4；string "hi"+NUL+pad = 4
        assert_eq!(p.len(), 16 + 4 + 4);
        assert!(p.starts_with(b"/chatbox/input\0\0"));
        assert_eq!(&p[16..20], b",sT\0");
        assert_eq!(&p[20..24], b"hi\0\0");
    }

    #[test]
    fn chatbox_bool_typetag_is_t() {
        let p = osc_chatbox_packet("");
        assert_eq!(&p[16..20], b",sT\0");
    }
}

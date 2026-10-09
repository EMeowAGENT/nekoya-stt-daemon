//! 假 WayVR IPC 服务（离线测试用）。
//!
//! 监听抽象 socket `\0/tmp/wayvr_ipc.sock`，完成握手后把收到的每个帧以一行 JSON
//! 打印到 stdout，供 `scripts/test/06-wayvr-mock.sh` 断言。

use std::io::Write;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

const SOCKET_NAME: &str = "/tmp/wayvr_ipc.sock";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let addr = SocketAddr::from_abstract_name(SOCKET_NAME.as_bytes())?;
    // std 绑定抽象名，再转 tokio。
    let std_listener = std::os::unix::net::UnixListener::bind_addr(&addr)?;
    std_listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(std_listener)?;
    eprintln!("mock_wayvr_ipc: listening on abstract {SOCKET_NAME}");

    loop {
        let (mut stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            loop {
                let mut header = [0u8; 4];
                if stream.read_exact(&mut header).await.is_err() {
                    return;
                }
                let len = u32::from_be_bytes(header) as usize;
                if len > 128 * 1024 {
                    return;
                }
                let mut payload = vec![0u8; len];
                if stream.read_exact(&mut payload).await.is_err() {
                    return;
                }
                let text = String::from_utf8_lossy(&payload).to_string();

                if text.contains("\"Handshake\"") {
                    let resp = br#"{"HandshakeSuccess":{"runtime":"wayvr-server"}}"#;
                    let mut frame = (resp.len() as u32).to_be_bytes().to_vec();
                    frame.extend_from_slice(resp);
                    let _ = stream.write_all(&frame).await;
                    eprintln!("mock_wayvr_ipc: handshake ok");
                } else {
                    println!("{text}");
                    let _ = std::io::stdout().flush();
                }
            }
        });
    }
}

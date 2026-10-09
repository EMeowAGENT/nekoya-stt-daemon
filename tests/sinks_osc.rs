//! OSC 发送的真实 UDP 集成测试（离线）。
use std::net::UdpSocket;
use std::time::Duration;

#[test]
fn send_osc_delivers_chatbox_packet() {
    let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    nekoya_stt::sinks::send_osc(port, "hi").unwrap();

    let mut buf = [0u8; 512];
    let n = listener.recv(&mut buf).unwrap();
    let packet = &buf[..n];
    assert!(packet.starts_with(b"/chatbox/input\0\0"), "地址前缀不符");
    assert!(packet.windows(4).any(|w| w == b",sT\0"), "typetag 应为 ,sT");
    assert!(packet.windows(4).any(|w| w == b"hi\0\0"), "文本载荷缺失");
}

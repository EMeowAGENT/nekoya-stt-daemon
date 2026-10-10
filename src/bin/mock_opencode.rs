//! 假 OpenCode HTTP 服务（离线测试用）。
//!
//! - `GET /api/session*` → 返回两条根会话
//! - `POST /api/session/{id}/prompt` → 把收到的 body 打印为 `PROMPT {path} {body}` 到 stdout，返回 200
//!
//! 端口：`argv[1]` 或环境变量 `MOCK_OPENCODE_PORT`，默认 49399。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("MOCK_OPENCODE_PORT").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(49399);
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("mock_opencode: listening on 127.0.0.1:{port}");

    for stream in listener.incoming() {
        let mut stream = stream?;
        let _ = handle(&mut stream);
    }
    Ok(())
}

fn handle(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body_str = String::from_utf8_lossy(&body).to_string();

    if method == "GET" && path.starts_with("/api/session/") {
        // 单会话查询：返回 agent，供"跟随 TUI 的 agent"测试。
        println!("GET1 {path}");
        let _ = std::io::stdout().flush();
        let json = r#"{"data":{"id":"ses_mock1","title":"Mock Session 1","agent":"plan","location":{"directory":"/tmp"},"time":{"updated":200}}}"#;
        write_resp(stream, 200, json)
    } else if method == "GET" && path.starts_with("/api/session") {
        println!("LIST {path}");
        let _ = std::io::stdout().flush();
        let json = r#"{"data":[{"id":"ses_mock1","title":"Mock Session 1","location":{"directory":"/tmp"},"time":{"updated":200}},{"id":"ses_mock2","title":"Mock Session 2","location":{"directory":"/tmp"},"time":{"updated":100}}],"cursor":{"previous":null,"next":null}}"#;
        write_resp(stream, 200, json)
    } else if method == "POST" && path.ends_with("/prompt") {
        println!("PROMPT {path} {body_str}");
        let _ = std::io::stdout().flush();
        write_resp(
            stream,
            200,
            r#"{"data":{"id":"msg_1","sessionID":"ses_mock1","time":{"created":1},"type":"user","payload":{"text":""},"delivery":"steer"}}"#,
        )
    } else {
        write_resp(stream, 404, r#"{"error":"not found"}"#)
    }
}

fn write_resp(stream: &mut TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes())?;
    stream.flush()
}

//! 极简阻塞式 HTTP/1.1 客户端（仅用于本机 OpenCode 服务，`http://`）。
//!
//! 不引入重依赖：手写请求/响应解析，支持 `Content-Length` 与 `Transfer-Encoding: chunked`。
//! 解析部分为纯函数，可离线单测。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{bail, Context};

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

/// 解析 `http://host[:port][/path?query]`。
pub fn parse_url(url: &str) -> anyhow::Result<Url> {
    let (scheme, rest) = url.split_once("://").context("URL 缺少 scheme")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" {
        bail!("仅支持 http:// 端点（got {scheme}）");
    }
    let (hostport, path) = match rest.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().context("端口解析失败")?),
        None => (hostport.to_string(), 80),
    };
    Ok(Url {
        scheme,
        host,
        port,
        path,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// 状态行 `HTTP/1.1 200 OK` → 200。
pub fn parse_status(line: &str) -> Option<u16> {
    let mut it = line.split_whitespace();
    let _http = it.next()?;
    it.next()?.parse::<u16>().ok()
}

/// 返回 header 段结束位置（即 `\r\n\r\n` 之后的下标）。
pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// 从 header 段里取某字段（大小写不敏感）。
pub fn header_value(headers: &str, name: &str) -> Option<String> {
    let lname = name.to_ascii_lowercase();
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(&lname) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// 解码 chunked body。
pub fn decode_chunked(mut body: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let Some(pos) = body.windows(2).position(|w| w == b"\r\n") else {
            // 没有更多 chunk 头
            break;
        };
        let size_line = std::str::from_utf8(&body[..pos]).unwrap_or("");
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).context("chunk 大小非法")?;
        body = &body[pos + 2..];
        if size == 0 {
            break;
        }
        if body.len() < size {
            bail!("chunk 数据不完整");
        }
        out.extend_from_slice(&body[..size]);
        body = &body[(size + 2).min(body.len())..]; // 跳过数据与其后的 CRLF
    }
    Ok(out)
}

/// 解析完整响应字节流。
pub fn parse_response(raw: &[u8]) -> anyhow::Result<Response> {
    let end = find_header_end(raw).context("响应缺少 header 结束符")?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let mut lines = head.lines();
    let status = parse_status(lines.next().unwrap_or("")).context("无法解析状态行")?;
    let headers = &head[head.find('\n').map(|i| i + 1).unwrap_or(0)..];
    let body = &raw[end..];

    let body = if let Some(te) = header_value(headers, "transfer-encoding") {
        if te.to_ascii_lowercase().contains("chunked") {
            decode_chunked(body)?
        } else {
            body.to_vec()
        }
    } else if let Some(cl) = header_value(headers, "content-length") {
        let n: usize = cl.parse().context("Content-Length 非法")?;
        body[..n.min(body.len())].to_vec()
    } else {
        body.to_vec()
    };

    Ok(Response { status, body })
}

/// 发送一次请求（`Connection: close`）。阻塞；调用方应在阻塞线程中执行。
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&[u8]>,
) -> anyhow::Result<Response> {
    let u = parse_url(url)?;
    let mut stream = TcpStream::connect((u.host.as_str(), u.port))
        .with_context(|| format!("连接 {}:{} 失败", u.host, u.port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let mut req = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n",
        u.path, u.host, u.port
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");

    stream.write_all(req.as_bytes()).context("写入请求失败")?;
    if let Some(b) = body {
        stream.write_all(b).context("写入请求体失败")?;
    }
    stream.flush().ok();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).context("读取响应失败")?;
    parse_response(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing() {
        let u = parse_url("http://127.0.0.1:49374").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("127.0.0.1", 49374, "/")
        );
        let u = parse_url("http://127.0.0.1:49374/api/session?order=desc").unwrap();
        assert_eq!(u.path, "/api/session?order=desc");
        let u = parse_url("http://x").unwrap();
        assert_eq!(u.port, 80);
        assert!(parse_url("https://x").is_err());
    }

    #[test]
    fn status_and_header_end() {
        assert_eq!(parse_status("HTTP/1.1 200 OK"), Some(200));
        assert_eq!(parse_status("HTTP/1.1 404 Not Found"), Some(404));
        assert_eq!(
            find_header_end(b"HTTP/1.1 200 OK\r\nA: b\r\n\r\nbody"),
            Some(25)
        );
    }

    #[test]
    fn header_lookup_case_insensitive() {
        let h = "Content-Type: application/json\r\nTransfer-Encoding: chunked\r\n";
        assert_eq!(
            header_value(h, "content-type").as_deref(),
            Some("application/json")
        );
        assert_eq!(
            header_value(h, "TRANSFER-ENCODING").as_deref(),
            Some("chunked")
        );
    }

    #[test]
    fn chunked_decoding() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"Wikipedia");
    }

    #[test]
    fn parse_content_length_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloextra";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn parse_chunked_body() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, b"abc");
    }
}

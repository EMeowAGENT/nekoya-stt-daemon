//! OpenCode 接入：端点发现、Basic 鉴权、会话列表、推送 prompt。
//!
//! 事实（针对 opencode v2.0.26）：
//! - 运行时注册：`$XDG_STATE_HOME/opencode/service.json` → `{ id, version, url, pid, password }`
//! - 配置：`$XDG_CONFIG_HOME/opencode/service.json` → `{ hostname, port, password, … }`（默认端口 49374）
//! - 鉴权：`Authorization: Basic base64(username + ":" + password)`，用户名默认 `opencode`
//! - 会话：`GET /api/session`（根会话 `parentID=null`，按 `time.updated` 倒序）
//! - 发送：`POST /api/session/{id}/prompt {"text": …}`
//! - **不自动拉起服务**；发现失败即报错、不推送。

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{config, http};

/// 解析出的端点。
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub url: String,
    pub username: String,
    pub password: Option<String>,
}

impl Endpoint {
    /// 构造 Basic 鉴权头值。
    pub fn basic_header(&self) -> Option<String> {
        let pw = self.password.as_ref()?;
        let raw = format!("{}:{}", self.username, pw);
        Some(format!("Basic {}", base64_encode::b64(raw.as_bytes())))
    }
}

/// 会话摘要（用于列表与选择）。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub directory: Option<String>,
    pub updated: i64,
}

impl SessionInfo {
    /// 面板行显示文案（截断，避免过长）。
    pub fn row_label(&self) -> String {
        let title = if self.title.trim().is_empty() {
            "(无标题)"
        } else {
            self.title.trim()
        };
        let title: String = title.chars().take(48).collect();
        let short_id: String = self.id.chars().take(10).collect();
        format!("{title}  [{short_id}]")
    }
}

#[derive(Deserialize)]
struct Registration {
    url: String,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct Settings {
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct SessionsResponse {
    data: Vec<RawSession>,
}

#[derive(Deserialize)]
struct RawSession {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    location: Option<RawLocation>,
    #[serde(default)]
    time: Option<RawTime>,
}

#[derive(Deserialize)]
struct RawLocation {
    #[serde(default)]
    directory: Option<String>,
}

#[derive(Deserialize)]
struct RawTime {
    #[serde(default)]
    updated: Option<i64>,
}

pub const DEFAULT_PORT: u16 = 49374;

/// 从运行时注册文件内容解析端点。
pub fn parse_registration(json: &str) -> Option<Endpoint> {
    let r: Registration = serde_json::from_str(json).ok()?;
    Some(Endpoint {
        url: r.url,
        username: "opencode".into(),
        password: r.password,
    })
}

/// 从配置(settings)文件内容解析 `(host, port, password)`。
pub fn parse_settings(json: &str) -> Option<(String, u16, Option<String>)> {
    let s: Settings = serde_json::from_str(json).ok()?;
    Some((
        s.hostname.unwrap_or_else(|| "127.0.0.1".into()),
        s.port.unwrap_or(DEFAULT_PORT),
        s.password,
    ))
}

/// 解析顺序：显式 url 覆盖 → 运行时注册 → 配置(settings)。
pub fn resolve(
    url_override: Option<&str>,
    password_override: Option<&str>,
    username: &str,
    reg: Option<&str>,
    settings: Option<&str>,
) -> Option<Endpoint> {
    if let Some(url) = url_override {
        return Some(Endpoint {
            url: url.to_string(),
            username: username.to_string(),
            password: password_override.map(str::to_string),
        });
    }
    if let Some(reg) = reg {
        if let Some(mut ep) = parse_registration(reg) {
            ep.username = username.to_string();
            if let Some(pw) = password_override {
                ep.password = Some(pw.to_string());
            }
            return Some(ep);
        }
    }
    if let Some(st) = settings {
        if let Some((host, port, pw)) = parse_settings(st) {
            return Some(Endpoint {
                url: format!("http://{host}:{port}"),
                username: username.to_string(),
                password: password_override.map(str::to_string).or(pw),
            });
        }
    }
    None
}

/// 读取本机注册/配置文件并解析端点。
pub fn discover(cfg: &config::Config) -> Option<Endpoint> {
    let reg = std::fs::read_to_string(config::opencode_registration_file()).ok();
    let settings = std::fs::read_to_string(config::opencode_settings_file()).ok();
    resolve(
        cfg.opencode.url.as_deref(),
        cfg.opencode.password.as_deref(),
        &cfg.opencode.username,
        reg.as_deref(),
        settings.as_deref(),
    )
}

/// 解析 `GET /api/session` 响应。
pub fn parse_sessions(json: &str) -> Result<Vec<SessionInfo>> {
    let resp: SessionsResponse = serde_json::from_str(json).context("解析会话列表失败")?;
    let mut out: Vec<SessionInfo> = resp
        .data
        .into_iter()
        .map(|s| SessionInfo {
            id: s.id,
            title: s.title.unwrap_or_default(),
            directory: s.location.and_then(|l| l.directory),
            updated: s.time.and_then(|t| t.updated).unwrap_or(0),
        })
        .collect();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    Ok(out)
}

fn auth_headers(ep: &Endpoint) -> Vec<(&'static str, String)> {
    let mut headers = vec![("Accept", "application/json".to_string())];
    if let Some(auth) = ep.basic_header() {
        headers.push(("Authorization", auth));
    }
    headers
}

/// 拉取根会话列表（最近 `limit` 条）。
pub fn list_sessions(
    ep: &Endpoint,
    limit: u32,
    directory: Option<&str>,
) -> Result<Vec<SessionInfo>> {
    let mut url = format!(
        "{}/api/session?order=desc&limit={limit}&parentID=null",
        ep.url
    );
    if let Some(dir) = directory {
        url.push_str(&format!("&directory={}", urlencode(dir)));
    }
    let headers = auth_headers(ep);
    let resp = http::request("GET", &url, &headers, None)?;
    if resp.status != 200 {
        anyhow::bail!(
            "GET /api/session 返回 {}: {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    parse_sessions(&String::from_utf8_lossy(&resp.body))
}

/// 向指定会话推送一条 prompt。
pub fn prompt(ep: &Endpoint, session_id: &str, text: &str) -> Result<()> {
    if !session_id.starts_with("ses") {
        anyhow::bail!("会话 id 非法: {session_id}");
    }
    let url = format!("{}/api/session/{}/prompt", ep.url, session_id);
    let headers = auth_headers(ep);
    let body = serde_json::json!({ "text": text }).to_string();
    let resp = http::request("POST", &url, &headers, Some(body.as_bytes()))?;
    if !(200..300).contains(&resp.status) {
        anyhow::bail!(
            "POST prompt 返回 {}: {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
    Ok(())
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

mod base64_encode {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn b64(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_takes_priority_over_settings() {
        let reg = r#"{"id":"x","version":"2.0.26","url":"http://127.0.0.1:49374","pid":1,"password":"pw"}"#;
        let set = r#"{"hostname":"127.0.0.1","port":1234,"password":"other"}"#;
        let ep = resolve(None, None, "opencode", Some(reg), Some(set)).unwrap();
        assert_eq!(ep.url, "http://127.0.0.1:49374");
        assert_eq!(ep.password.as_deref(), Some("pw"));
    }

    #[test]
    fn settings_default_port_and_host() {
        let ep = resolve(None, None, "opencode", None, Some(r#"{"password":"pw"}"#)).unwrap();
        assert_eq!(ep.url, "http://127.0.0.1:49374");
    }

    #[test]
    fn url_override_and_env_password() {
        let ep = resolve(Some("http://x:9"), Some("pw2"), "opencode", None, None).unwrap();
        assert_eq!(ep.url, "http://x:9");
        assert_eq!(ep.password.as_deref(), Some("pw2"));
    }

    #[test]
    fn basic_header_matches_documented_shape() {
        let ep = Endpoint {
            url: "http://127.0.0.1:49374".into(),
            username: "opencode".into(),
            password: Some("secret".into()),
        };
        assert_eq!(ep.basic_header().unwrap(), "Basic b3BlbmNvZGU6c2VjcmV0");
    }

    #[test]
    fn no_source_yields_none() {
        assert!(resolve(None, None, "opencode", None, None).is_none());
    }

    #[test]
    fn parse_sessions_sorts_by_updated_desc() {
        let json = r#"{"data":[
            {"id":"ses_b","title":"B","location":{"directory":"/b"},"time":{"updated":100}},
            {"id":"ses_a","title":"A","location":{"directory":"/a"},"time":{"updated":200}}
        ],"cursor":{"previous":null,"next":null}}"#;
        let list = parse_sessions(json).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "ses_a");
        assert_eq!(list[0].directory.as_deref(), Some("/a"));
    }

    #[test]
    fn row_label_truncates() {
        let s = SessionInfo {
            id: "ses_0123456789".into(),
            title: "x".repeat(80),
            directory: None,
            updated: 0,
        };
        let label = s.row_label();
        assert!(label.contains("[ses_012345"));
        assert!(label.chars().count() < 70);
    }

    #[test]
    fn parse_sessions_handles_missing_fields() {
        let json = r#"{"data":[{"id":"ses_x"}]}"#;
        let list = parse_sessions(json).unwrap();
        assert_eq!(list[0].title, "");
        assert_eq!(list[0].updated, 0);
    }
}

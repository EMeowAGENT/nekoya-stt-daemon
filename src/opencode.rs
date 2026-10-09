//! OpenCode 端点发现与 Basic 鉴权（纯解析，便于单测）。
//!
//! 事实（针对 opencode v2.0.26）：
//! - 运行时注册：`$XDG_STATE_HOME/opencode/service.json` → `{ id, version, url, pid, password }`
//! - 配置：`$XDG_CONFIG_HOME/opencode/service.json` → `{ hostname, port, password, … }`（默认端口 49374）
//! - 鉴权：`Authorization: Basic base64(username + ":" + password)`，用户名默认 `opencode`
//! - **不自动拉起服务**；发现失败即报错、不推送。

use base64_encode::b64;
use serde::Deserialize;

/// 解析出的端点。
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub url: String,
    pub username: String,
    pub password: Option<String>,
}

impl Endpoint {
    /// 构造 Basic 鉴权头值（不含 "Basic " 前缀之外的编码细节）。
    pub fn basic_header(&self) -> Option<String> {
        let pw = self.password.as_ref()?;
        let raw = format!("{}:{}", self.username, pw);
        Some(format!("Basic {}", b64(raw.as_bytes())))
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
/// `reg` / `settings` 为对应文件内容（不存在则传 None）。
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

/// 供后续阶段使用的最小 base64 实现（避免额外依赖）。
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
}

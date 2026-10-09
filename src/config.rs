//! 配置加载（`~/.config/nekoya-stt/config.toml`）+ 环境变量覆盖。
//!
//! 密钥**只**来自环境变量 `DASHSCOPE_API_KEY`，绝不写入仓库/配置文件。

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub audio: Audio,
    pub qwen: Qwen,
    pub corpus: Corpus,
    /// 本地 ITN 后处理开关（v1 未实现规则，默认关闭）。
    pub itn: bool,
    pub osc: Osc,
    pub wayvr: Wayvr,
    pub opencode: Opencode,
    pub ptt: Ptt,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Ptt {
    /// 手表键：按住超过该毫秒数才开始提交 STT（此前仅本地录制缓冲）。
    /// 同时用作"短按丢弃"的阈值。
    pub start_after_ms: u64,
}

impl Default for Ptt {
    fn default() -> Self {
        Self {
            start_after_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Audio {
    /// 指定 PipeWire 输入源名；None = 跟随系统默认。
    pub source: Option<String>,
    pub sample_rate: u32,
    /// 预滚缓冲（毫秒），避免吞掉首字。
    pub preroll_ms: u64,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            source: None,
            sample_rate: 16_000,
            preroll_ms: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Qwen {
    pub model: String,
    pub endpoint: String,
    /// None = 自动语种。
    pub language: Option<String>,
}

impl Default for Qwen {
    fn default() -> Self {
        Self {
            model: "qwen3-asr-flash-realtime".to_string(),
            endpoint: "wss://dashscope.aliyuncs.com/api-ws/v1/realtime".to_string(),
            language: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Corpus {
    /// 是否把 `lexicon.txt` 作为 `input_audio_transcription.corpus.text` 发送。
    pub enabled: bool,
}

impl Default for Corpus {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Osc {
    pub port: u16,
    pub chatbox_max_chars: usize,
}

impl Default for Osc {
    fn default() -> Self {
        Self {
            port: 9000,
            chatbox_max_chars: 144,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Wayvr {
    /// 默认禁用：WayVR 不一定在运行。
    pub enabled: bool,
    /// 客户端名（1..=31 字节）。
    pub client_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Opencode {
    /// 覆盖 URL；否则自动发现。
    pub url: Option<String>,
    pub username: String,
    /// 覆盖密码；默认从 service.json 读取。
    pub password: Option<String>,
    /// 会话列表可选按目录过滤。
    pub directory: Option<String>,
}

impl Default for Opencode {
    fn default() -> Self {
        Self {
            url: None,
            username: "opencode".to_string(),
            password: None,
            directory: None,
        }
    }
}

impl Config {
    /// 应用环境变量覆盖（`NEKOYA_*`）。
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var(crate::env::MODEL) {
            if !v.is_empty() {
                self.qwen.model = v;
            }
        }
        if let Ok(v) = std::env::var(crate::env::VRC_OSC_PORT) {
            if let Ok(p) = v.parse() {
                self.osc.port = p;
            }
        }
        if let Ok(v) = std::env::var(crate::env::OPENCODE_URL) {
            if !v.is_empty() {
                self.opencode.url = Some(v);
            }
        }
        if let Ok(v) = std::env::var(crate::env::OPENCODE_PASSWORD) {
            if !v.is_empty() {
                self.opencode.password = Some(v);
            }
        }
        if let Ok(v) = std::env::var(crate::env::WAYVR_ENABLED) {
            self.wayvr.enabled = matches!(v.as_str(), "1" | "true" | "yes" | "on");
        }
        self
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let cfg = if path.is_file() {
            let text = std::fs::read_to_string(path)?;
            toml::from_str::<Config>(&text)?
        } else {
            Config::default()
        };
        Ok(cfg.with_env_overrides())
    }

    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&config_file())
    }

    /// 密钥来源：仅环境变量。
    pub fn api_key() -> Option<String> {
        std::env::var(crate::env::DASHSCOPE_API_KEY)
            .ok()
            .filter(|s| !s.is_empty())
    }
}

// ---- 路径 ----

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn xdg_config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

pub fn xdg_runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn config_dir() -> PathBuf {
    xdg_config_home().join("nekoya-stt")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_file() -> PathBuf {
    config_dir().join("state.toml")
}

pub fn lexicon_file() -> PathBuf {
    config_dir().join("lexicon.txt")
}

pub fn control_socket() -> PathBuf {
    xdg_runtime_dir().join("nekoya-stt.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative() {
        let c = Config::default();
        assert!(!c.wayvr.enabled, "WayVR 必须默认禁用");
        assert!(!c.itn, "ITN 默认关闭");
        assert!(c.corpus.enabled);
        assert_eq!(c.osc.port, 9000);
        assert_eq!(c.osc.chatbox_max_chars, 144);
        assert_eq!(c.qwen.model, "qwen3-asr-flash-realtime");
    }

    #[test]
    fn parses_partial_toml() {
        let c: Config = toml::from_str(
            r#"
            itn = true
            [wayvr]
            enabled = true
            "#,
        )
        .unwrap();
        assert!(c.itn);
        assert!(c.wayvr.enabled);
        assert_eq!(c.osc.port, 9000);
    }
}

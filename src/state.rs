//! 持久化状态（`~/.config/nekoya-stt/state.toml`）。
//!
//! 只保存「已选 OpenCode 会话 id」。**不保存任何转写文本**。

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// 已选会话 id（`ses…`）。
    pub session: Option<String>,
}

impl State {
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&crate::config::state_file())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&crate::config::state_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_no_transcript_field() {
        let s = State {
            session: Some("ses_abc".into()),
        };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("ses_abc"));
        // 状态里绝不应出现转写相关字段
        assert!(!text.contains("transcript"));
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn missing_file_is_default() {
        let tmp = std::env::temp_dir().join("nekoya-nonexistent-state.toml");
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(State::load_from(&tmp).unwrap(), State::default());
    }
}

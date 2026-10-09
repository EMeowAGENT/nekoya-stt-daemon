//! 识别会话状态机（纯逻辑，无 IO）。
//!
//! 规则：
//! - 单会话锁：识别进行中忽略一切新的开始请求（先到先得）。
//! - 只保留「最近一次终稿」，不保留历史（不存在"整段会话"）。
//! - 终稿只驻留内存，不落盘（由调用方保证）。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// 空闲。
    #[default]
    Idle,
    /// 主面板 PTT 手动模式（按住说话，松手 commit/finish）。
    PttListening,
    /// 手表长按：手动 VAD 自动模式，说完自动收尾。
    AutoListening,
}

impl Phase {
    pub fn is_listening(self) -> bool {
        !matches!(self, Phase::Idle)
    }

    /// 面板状态文案（中文）。
    pub fn label(self) -> &'static str {
        match self {
            Phase::Idle => "空闲",
            Phase::PttListening => "聆听中…",
            Phase::AutoListening => "自动聆听中…",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// 已在识别中，忽略本次开始。
    Busy,
}

#[derive(Debug, Default)]
pub struct Engine {
    phase: Phase,
    last_final: Option<String>,
    /// 识别中的实时预览（text + stash 拼接）。
    preview: String,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            last_final: None,
            preview: String::new(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 最近一次终稿（只此一份，无历史）。
    pub fn last_final(&self) -> Option<&str> {
        self.last_final.as_deref()
    }

    /// 当前实时预览（识别中 text+stash；空闲时为空）。
    pub fn preview(&self) -> &str {
        &self.preview
    }

    /// 主面板 PTT 按下。
    pub fn start_ptt(&mut self) -> Result<(), StartError> {
        if self.phase.is_listening() {
            return Err(StartError::Busy);
        }
        self.phase = Phase::PttListening;
        self.preview.clear();
        Ok(())
    }

    /// 手表长按触发的自动识别。
    pub fn start_auto(&mut self) -> Result<(), StartError> {
        if self.phase.is_listening() {
            return Err(StartError::Busy);
        }
        self.phase = Phase::AutoListening;
        self.preview.clear();
        Ok(())
    }

    /// 接收实时预览（服务端 `text` + `stash` 拼接后的完整预览句）。
    pub fn on_preview(&mut self, text: impl Into<String>) {
        self.preview = text.into();
    }

    /// 结束识别。`final_text` 为终稿（可能为空：没有检测到语音）。
    /// 返回是否产生了新的终稿。
    pub fn finish(&mut self, final_text: Option<String>) -> bool {
        self.phase = Phase::Idle;
        self.preview.clear();
        match final_text {
            Some(t) if !t.trim().is_empty() => {
                self.last_final = Some(t);
                true
            }
            _ => false,
        }
    }

    /// 是否命中 55s 手动上限（调用方据采样时长判断后触发强制收尾）。
    pub const MANUAL_SOFT_LIMIT_SECS: u64 = 55;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_session_lock() {
        let mut e = Engine::new();
        assert_eq!(e.start_ptt(), Ok(()));
        assert_eq!(e.start_auto(), Err(StartError::Busy));
        assert_eq!(e.start_ptt(), Err(StartError::Busy));
        assert!(e.finish(Some("hi".into())));
        // 结束后可再次开始
        assert_eq!(e.start_auto(), Ok(()));
    }

    #[test]
    fn keeps_only_last_final() {
        let mut e = Engine::new();
        e.start_ptt().unwrap();
        e.finish(Some("第一条".into()));
        e.start_ptt().unwrap();
        e.finish(Some("第二条".into()));
        assert_eq!(e.last_final(), Some("第二条"));
    }

    #[test]
    fn empty_final_does_not_overwrite() {
        let mut e = Engine::new();
        e.start_ptt().unwrap();
        e.finish(Some("保留我".into()));
        e.start_auto().unwrap();
        assert!(!e.finish(Some("   ".into())));
        assert_eq!(e.last_final(), Some("保留我"));
        assert_eq!(e.phase(), Phase::Idle);
    }

    #[test]
    fn preview_cleared_on_finish() {
        let mut e = Engine::new();
        e.start_ptt().unwrap();
        e.on_preview("你好世");
        assert_eq!(e.preview(), "你好世");
        e.finish(Some("你好世界".into()));
        assert_eq!(e.preview(), "");
    }
}

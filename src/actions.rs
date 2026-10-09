//! 面板按钮 → 动作（纯逻辑，无 IO）。
//!
//! 主面板按钮（与 docs/architecture.md 对应）：
//! `0 PTT` · `1 复制到剪贴板` · `2 VRChat OSC` · `3 Ctrl+V` · `4 回车`
//! `5 推送 OpenCode 并发送` · `6 会话列表`
//!
//! 约束：
//! - 按钮 1/2/3/5 作用于**最近一次终稿**；无终稿 → 仅提示状态。
//! - 按钮 5 必须有已选会话，否则拒绝且不新建。

pub const VRC_OSC_CHATBOX_MAX: usize = 144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Copy,
    Osc,
    CtrlV,
    Enter,
    PushOpencode,
    SessionList,
}

impl Button {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "copy" => Button::Copy,
            "osc" => Button::Osc,
            "ctrl-v" | "ctrl_v" | "paste" => Button::CtrlV,
            "enter" => Button::Enter,
            "push" => Button::PushOpencode,
            "list" => Button::SessionList,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 写宿主 Wayland 剪贴板（`wl-copy`）。
    CopyToClipboard(String),
    /// 经 VRChat OSC 发送（`/chatbox/input`，已按上限截断）。
    SendOsc(String),
    /// 用 WayVR 虚拟键盘路由注入 Ctrl+V。
    SendCtrlV,
    /// 用 WayVR 虚拟键盘路由注入回车。
    SendEnter,
    /// 推送到指定 OpenCode 会话并发送。
    PushOpencode { session: String, text: String },
    /// 展开会话列表（daemon 拉取列表并写回面板）。
    ShowSessionList,
    /// 仅在面板显示一条状态（不产生副作用）。
    Status(String),
}

/// 计算某个按钮应产生的动作序列。
pub fn dispatch(
    button: Button,
    last_final: Option<&str>,
    session: Option<&str>,
    osc_max: usize,
) -> Vec<Action> {
    match button {
        Button::SessionList => vec![Action::ShowSessionList],
        Button::Enter => vec![Action::SendEnter],
        _ => {
            let Some(text) = last_final else {
                return vec![Action::Status("无可用转写".into())];
            };
            match button {
                Button::Copy => vec![Action::CopyToClipboard(text.to_string())],
                Button::Osc => vec![Action::SendOsc(truncate_chars(text, osc_max))],
                Button::CtrlV => vec![Action::SendCtrlV],
                Button::PushOpencode => match session {
                    Some(sid) if !sid.is_empty() => vec![Action::PushOpencode {
                        session: sid.to_string(),
                        text: text.to_string(),
                    }],
                    _ => vec![Action::Status("未选择会话".into())],
                },
                Button::Enter | Button::SessionList => unreachable!(),
            }
        }
    }
}

/// 按**字符**（非字节）截断，避免切断多字节字符。
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_osc_use_last_final() {
        assert_eq!(
            dispatch(Button::Copy, Some("你好"), None, 144),
            vec![Action::CopyToClipboard("你好".into())]
        );
        assert_eq!(
            dispatch(Button::Osc, Some("你好"), None, 144),
            vec![Action::SendOsc("你好".into())]
        );
    }

    #[test]
    fn osc_truncates_to_144_chars() {
        let long: String = "字".repeat(200);
        let got = dispatch(Button::Osc, Some(&long), None, VRC_OSC_CHATBOX_MAX);
        match &got[0] {
            Action::SendOsc(t) => assert_eq!(t.chars().count(), 144),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn push_requires_session_and_never_autocreates() {
        let got = dispatch(Button::PushOpencode, Some("hi"), None, 144);
        assert_eq!(got, vec![Action::Status("未选择会话".into())]);
        let got = dispatch(Button::PushOpencode, Some("hi"), Some("ses_1"), 144);
        assert_eq!(
            got,
            vec![Action::PushOpencode {
                session: "ses_1".into(),
                text: "hi".into()
            }]
        );
    }

    #[test]
    fn no_transcript_only_status() {
        for b in [
            Button::Copy,
            Button::Osc,
            Button::CtrlV,
            Button::PushOpencode,
        ] {
            assert_eq!(
                dispatch(b, None, Some("ses_1"), 144),
                vec![Action::Status("无可用转写".into())]
            );
        }
    }

    #[test]
    fn enter_and_list_need_no_transcript() {
        assert_eq!(
            dispatch(Button::Enter, None, None, 144),
            vec![Action::SendEnter]
        );
        assert_eq!(
            dispatch(Button::SessionList, None, None, 144),
            vec![Action::ShowSessionList]
        );
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate_chars("中文", 1), "中");
    }
}

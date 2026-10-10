//! WayVR 字幕面板（HUD）常量与构造（纯逻辑）。
//!
//! 面板由 WayVR 通过 `custom_panels` 创建（模板见 `wayvr/theme/gui/nekoya_stt.xml`）。
//! daemon 只通过 IPC `WlxModifyPanel` 改**文字元素**，绝不碰 transform/positioning/global。

use crate::wayvr_ipc::{PacketClient, WlxModifyPanelCommand, WlxModifyPanelParams};

/// 面板 overlay 名（= XML 文件名去扩展名）。
pub const PANEL: &str = "nekoya_stt";

/// 元素 id（需与 nekoya_stt.xml 一致）。
pub const EL_TRANSCRIPT: &str = "transcript";
pub const EL_STATUS: &str = "status";
/// 顶部"当前会话名称"标签（显示标题，不显示 id）。
pub const EL_SESSION: &str = "session";

/// 会话列表区容器 id 与行槽。
pub const EL_LIST: &str = "session_list";
/// 可见行数（多则卡；16 与"可视区"一致）。
pub const LIST_ROWS: usize = 16;
pub fn list_row_id(i: usize) -> String {
    format!("session_row_{i}")
}
/// 行内文本 label 的 id（SetText 打这里，便于控制字号/高度）。
pub fn list_row_text_id(i: usize) -> String {
    format!("session_row_{i}_text")
}

/// 构造「设置某元素文字」的 IPC 包。
pub fn set_text(element: &str, text: &str) -> PacketClient {
    PacketClient::WlxModifyPanel(WlxModifyPanelParams {
        overlay: PANEL.to_string(),
        element: element.to_string(),
        command: WlxModifyPanelCommand::SetText(text.to_string()),
    })
}

/// 构造「设置某元素可见性」的 IPC 包。
pub fn set_visible(element: &str, visible: bool) -> PacketClient {
    PacketClient::WlxModifyPanel(WlxModifyPanelParams {
        overlay: PANEL.to_string(),
        element: element.to_string(),
        command: WlxModifyPanelCommand::SetVisible(visible),
    })
}

/// 构造「确保面板显示/隐藏」的 IPC 包（幂等，非 toggle）。
pub fn set_panel_visible(visible: bool) -> PacketClient {
    PacketClient::WlxOverlaySetVisible(PANEL.to_string(), visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wayvr_ipc::encode;

    #[test]
    fn set_text_targets_panel() {
        let framed = encode(&set_text(EL_TRANSCRIPT, "hi")).unwrap();
        let json = String::from_utf8(framed[4..].to_vec()).unwrap();
        assert_eq!(
            json,
            r#"{"WlxModifyPanel":{"overlay":"nekoya_stt","element":"transcript","command":{"SetText":"hi"}}}"#
        );
    }

    #[test]
    fn panel_visible_is_ensure_visible() {
        let framed = encode(&set_panel_visible(true)).unwrap();
        let json = String::from_utf8(framed[4..].to_vec()).unwrap();
        assert_eq!(json, r#"{"WlxOverlaySetVisible":["nekoya_stt",true]}"#);
    }

    #[test]
    fn list_row_ids() {
        assert_eq!(list_row_id(0), "session_row_0");
        assert_eq!(list_row_id(15), "session_row_15");
        assert_eq!(list_row_text_id(3), "session_row_3_text");
        assert_eq!(LIST_ROWS, 16);
    }
}

//! WayVR IPC 协议（自实现，不依赖 WayVR 源码）。
//!
//! 事实（逆向自 wayvr 26.8.0 / 协议 v5）：
//! - socket 为 Linux **抽象命名空间**：名字字节 `\0/tmp/wayvr_ipc.sock`（前导 NUL）。
//! - 分帧：`u32 大端长度前缀 + UTF-8 JSON`，单包上限 128 KiB。
//! - 首包应为 `Handshake`；`header+payload` 必须一次性写出。
//! - 枚举使用 serde 默认「外部标签」：newtype 变体 → `{"Variant": payload}`，
//!   元组变体 → `{"Variant": [a, b]}`，单元变体 → `"Variant"`。
//!
//! 本模块只含**纯编解码**（可离线单测）；真正的 socket 连接在后续阶段实现。

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

pub mod conn;

pub const PROTOCOL_VERSION: u32 = 5;
pub const CONNECTION_MAGIC: &str = "wayvr_ipc";
/// WayVR 抽象 socket 名（前导 NUL 由连接层补上）。
pub const SOCKET_NAME: &str = "/tmp/wayvr_ipc.sock";
/// 单包上限（两端都强制）。
pub const MAX_PACKET: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handshake {
    pub protocol_version: u32,
    pub magic: String,
    pub client_name: String,
}

impl Handshake {
    pub fn new(client_name: impl Into<String>) -> anyhow::Result<Self> {
        let client_name = client_name.into();
        if client_name.is_empty() || client_name.len() >= 32 {
            bail!(
                "client_name must be 1..=31 bytes, got {}",
                client_name.len()
            );
        }
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            magic: CONNECTION_MAGIC.to_string(),
            client_name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WlxModifyPanelCommand {
    SetText(String),
    SetColor(String),
    SetImage(String),
    SetVisible(bool),
    SetStickyState(bool),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WlxModifyPanelParams {
    pub overlay: String,
    pub element: String,
    pub command: WlxModifyPanelCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxWindowStateField {
    Alpha,
    Grabbable,
    Interactable,
    Positioning,
    Curvature,
    Additive,
    BlockInput,
    AlignToHmd,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WlxHand {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WlxPositioning {
    Floating,
    Anchored,
    Static,
    FollowHead { lerp: f32 },
    FollowHand { hand: WlxHand, lerp: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WlxWindowStateValue {
    Bool(bool),
    Float(f32),
    Positioning(WlxPositioning),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WlxWindowStateSetParams {
    pub overlay: String,
    pub field: WlxWindowStateField,
    pub value: WlxWindowStateValue,
}

/// 客户端 → 服务端报文。serde 默认外部标签，形状与 WayVR 一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PacketClient {
    Handshake(Handshake),
    /// 注意：元组变体 → `{"WlxOverlaySetVisible":["name", true]}`
    WlxOverlaySetVisible(String, bool),
    WlxModifyPanel(WlxModifyPanelParams),
    WlxWindowStateSet(WlxWindowStateSetParams),
}

/// 服务端 → 客户端：我们只关心握手结果与断开，其余忽略。
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    HandshakeSuccess { runtime: String },
    Disconnect { reason: String },
    Other,
}

/// 编码一帧：`u32 BE 长度 + JSON`。
pub fn encode<P: Serialize>(packet: &P) -> anyhow::Result<Vec<u8>> {
    let json = serde_json::to_vec(packet).context("serialize packet")?;
    if json.len() > MAX_PACKET {
        bail!("packet too large: {} > {}", json.len(), MAX_PACKET);
    }
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

/// 解析 JSON payload（不含长度前缀）为服务端事件。
pub fn parse_server_payload(payload: &[u8]) -> anyhow::Result<ServerEvent> {
    let value: serde_json::Value =
        serde_json::from_slice(payload).context("decode server payload")?;
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Ok(ServerEvent::Other),
    };
    if let Some(hs) = obj.get("HandshakeSuccess") {
        let runtime = hs
            .get("runtime")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        return Ok(ServerEvent::HandshakeSuccess { runtime });
    }
    if let Some(dc) = obj.get("Disconnect") {
        let reason = dc
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        return Ok(ServerEvent::Disconnect { reason });
    }
    Ok(ServerEvent::Other)
}

/// 从 4 字节大端头读出 payload 长度并做上限校验。
pub fn payload_len_from_header(header: [u8; 4]) -> anyhow::Result<usize> {
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_PACKET {
        bail!("packet size too large (> {MAX_PACKET})");
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_of<P: Serialize>(p: &P) -> String {
        // encode() 在末尾追加长度前缀前的 JSON
        let framed = encode(p).unwrap();
        let len = u32::from_be_bytes(framed[..4].try_into().unwrap()) as usize;
        assert_eq!(len, framed.len() - 4);
        String::from_utf8(framed[4..].to_vec()).unwrap()
    }

    #[test]
    fn handshake_json_exact() {
        let hs = Handshake::new("nekoya-stt-1").unwrap();
        assert_eq!(
            json_of(&PacketClient::Handshake(hs)),
            r#"{"Handshake":{"protocol_version":5,"magic":"wayvr_ipc","client_name":"nekoya-stt-1"}}"#
        );
    }

    #[test]
    fn client_name_bounds() {
        assert!(Handshake::new("").is_err());
        assert!(Handshake::new("x".repeat(32)).is_err());
        assert!(Handshake::new("x".repeat(31)).is_ok());
    }

    #[test]
    fn overlay_set_visible_is_tuple_array() {
        let p = PacketClient::WlxOverlaySetVisible("nekoya_stt".into(), true);
        assert_eq!(
            json_of(&p),
            r#"{"WlxOverlaySetVisible":["nekoya_stt",true]}"#
        );
    }

    #[test]
    fn modify_panel_set_text_exact() {
        let p = PacketClient::WlxModifyPanel(WlxModifyPanelParams {
            overlay: "nekoya_stt".into(),
            element: "transcript".into(),
            command: WlxModifyPanelCommand::SetText("你好".into()),
        });
        assert_eq!(
            json_of(&p),
            r#"{"WlxModifyPanel":{"overlay":"nekoya_stt","element":"transcript","command":{"SetText":"你好"}}}"#
        );
    }

    #[test]
    fn window_state_global_bool_exact() {
        let p = PacketClient::WlxWindowStateSet(WlxWindowStateSetParams {
            overlay: "nekoya_stt".into(),
            field: WlxWindowStateField::Global,
            value: WlxWindowStateValue::Bool(true),
        });
        assert_eq!(
            json_of(&p),
            r#"{"WlxWindowStateSet":{"overlay":"nekoya_stt","field":"Global","value":{"Bool":true}}}"#
        );
    }

    #[test]
    fn window_state_positioning_exact() {
        let p = PacketClient::WlxWindowStateSet(WlxWindowStateSetParams {
            overlay: "x".into(),
            field: WlxWindowStateField::Positioning,
            value: WlxWindowStateValue::Positioning(WlxPositioning::FollowHand {
                hand: WlxHand::Right,
                lerp: 0.1,
            }),
        });
        assert_eq!(
            json_of(&p),
            r#"{"WlxWindowStateSet":{"overlay":"x","field":"Positioning","value":{"Positioning":{"FollowHand":{"hand":"Right","lerp":0.1}}}}}"#
        );
    }

    #[test]
    fn parse_handshake_success_and_disconnect() {
        let hs =
            parse_server_payload(br#"{"HandshakeSuccess":{"runtime":"wayvr-server"}}"#).unwrap();
        assert_eq!(
            hs,
            ServerEvent::HandshakeSuccess {
                runtime: "wayvr-server".into()
            }
        );
        let dc = parse_server_payload(br#"{"Disconnect":{"reason":"Invalid magic"}}"#).unwrap();
        assert_eq!(
            dc,
            ServerEvent::Disconnect {
                reason: "Invalid magic".into()
            }
        );
        let other = parse_server_payload(br#"{"WvrStateChanged":"WindowCreated"}"#).unwrap();
        assert_eq!(other, ServerEvent::Other);
    }

    #[test]
    fn header_limits() {
        assert_eq!(payload_len_from_header([0, 0, 0, 10]).unwrap(), 10);
        let too_big = (MAX_PACKET as u32 + 1).to_be_bytes();
        assert!(payload_len_from_header(too_big).is_err());
    }

    #[test]
    fn reject_oversized_packet() {
        let p = PacketClient::WlxOverlaySetVisible("x".repeat(MAX_PACKET), true);
        assert!(encode(&p).is_err());
    }
}

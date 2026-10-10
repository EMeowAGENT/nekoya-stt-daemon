//! 千问实时语音识别（qwen3-asr-flash-realtime，OpenAI-Realtime WS 协议）。
//!
//! 纯协议部分（`build_*` / `parse_event`）可离线单测；`transcribe` 为真实 WS 客户端。
//!
//! 事实要点（见 docs/architecture.md）：
//! - 端点 `wss://…/api-ws/v1/realtime?model=…`，鉴权 `Authorization: Bearer <key>`。
//! - 音频为 **base64 JSON** 的 `input_audio_buffer.append`，建议 3200B≈100ms/块。
//! - Manual 模式：`turn_detection: null` + `input_audio_buffer.commit`。
//! - 收尾必须 `session.finish` 并等 `session.finished`。
//! - 预览 = `text` + `stash`；终稿在 `…transcription.completed.transcript`。

use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 主面板 PTT：手动 commit 断句。
    Manual,
    /// 手表长按：服务端 VAD 自动断句。
    Vad,
}

#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub language: Option<String>,
    /// 专有名词 / 领域上下文（corpus.text）。
    pub corpus: Option<String>,
    /// VAD 静音阈值（毫秒）。
    pub vad_silence_ms: u32,
    pub vad_threshold: f32,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            endpoint: "wss://dashscope.aliyuncs.com/api-ws/v1/realtime".into(),
            model: "qwen3-asr-flash-realtime".into(),
            api_key: String::new(),
            language: None,
            corpus: None,
            vad_silence_ms: 800,
            vad_threshold: 0.2,
        }
    }
}

impl AsrConfig {
    /// 组装带 `?model=` 的完整 URL（保留已有 query）。
    pub fn url(&self) -> String {
        let sep = if self.endpoint.contains('?') { '&' } else { '?' };
        format!("{}{sep}model={}", self.endpoint, self.model)
    }
}

/// 服务端 → 客户端（我们关心的事件）。
#[derive(Debug, Clone, PartialEq)]
pub enum AsrEvent {
    /// `session.created` / `session.updated`。
    SessionReady,
    /// 实时预览（`text` + `stash` 拼接）。
    Preview(String),
    /// 终稿。
    Final(String),
    /// 错误。
    Error(String),
    /// `session.finished`。
    Finished,
    /// 其它事件（忽略）。
    Other,
}

// ---------------- 纯协议：构造 ----------------

fn event_id() -> String {
    // 简单、无依赖的唯一 id 生成。
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("event_{t}_{n}")
}

/// `session.update`（Manual：`turn_detection: null`；VAD：对象）。
pub fn build_session_update(cfg: &AsrConfig, mode: Mode) -> String {
    let mut transcription = serde_json::Map::new();
    if let Some(lang) = &cfg.language {
        transcription.insert("language".into(), json!(lang));
    }
    if let Some(corpus) = &cfg.corpus {
        if !corpus.trim().is_empty() {
            transcription.insert("corpus".into(), json!({ "text": corpus }));
        }
    }

    let turn_detection = match mode {
        Mode::Manual => Value::Null,
        Mode::Vad => json!({
            "type": "server_vad",
            "threshold": cfg.vad_threshold,
            "silence_duration_ms": cfg.vad_silence_ms,
        }),
    };

    json!({
        "event_id": event_id(),
        "type": "session.update",
        "session": {
            "input_audio_format": "pcm",
            "sample_rate": SAMPLE_RATE,
            "input_audio_transcription": Value::Object(transcription),
            "turn_detection": turn_detection,
        }
    })
    .to_string()
}

/// `input_audio_buffer.append`（base64 PCM16 LE）。
pub fn build_append(pcm: &[u8]) -> String {
    let audio = base64::engine::general_purpose::STANDARD.encode(pcm);
    json!({
        "event_id": event_id(),
        "type": "input_audio_buffer.append",
        "audio": audio,
    })
    .to_string()
}

pub fn build_commit() -> String {
    json!({ "event_id": event_id(), "type": "input_audio_buffer.commit" }).to_string()
}

pub fn build_finish() -> String {
    json!({ "event_id": event_id(), "type": "session.finish" }).to_string()
}

// ---------------- 纯协议：解析 ----------------

pub fn parse_event(text: &str) -> AsrEvent {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return AsrEvent::Other;
    };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    match ty {
        "session.created" | "session.updated" => AsrEvent::SessionReady,
        "conversation.item.input_audio_transcription.text" => {
            let t = v.get("text").and_then(Value::as_str).unwrap_or("");
            let stash = v.get("stash").and_then(Value::as_str).unwrap_or("");
            AsrEvent::Preview(format!("{t}{stash}"))
        }
        "conversation.item.input_audio_transcription.completed" => {
            let t = v.get("transcript").and_then(Value::as_str).unwrap_or("");
            AsrEvent::Final(t.to_string())
        }
        "conversation.item.input_audio_transcription.failed" => {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("transcription failed");
            AsrEvent::Error(msg.to_string())
        }
        "error" => {
            let msg = v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| v.get("message").and_then(Value::as_str))
                .unwrap_or("unknown error");
            AsrEvent::Error(msg.to_string())
        }
        "session.finished" => AsrEvent::Finished,
        _ => AsrEvent::Other,
    }
}

// ---------------- 真实 WS 客户端 ----------------

/// 一次「一说一识别」会话：从 `audio` 收 PCM 块持续发送，音频通道关闭后收尾，
/// 事件（预览/终稿/错误）经 `events` 送出。
pub async fn transcribe(
    cfg: AsrConfig,
    mode: Mode,
    mut audio: mpsc::Receiver<Vec<u8>>,
    events: mpsc::UnboundedSender<AsrEvent>,
) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let mut request = cfg.url().into_client_request().context("build ws request")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", cfg.api_key).parse().context("auth header")?,
    );

    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .context("connect qwen realtime ws")?;

    // 拆成写端/读端：边发音频边收实时预览（否则预览只能等到松手后才到达）。
    let (mut sink, mut stream) = ws.split();

    // 配置会话
    sink.send(Message::text(build_session_update(&cfg, mode)))
        .await
        .context("send session.update")?;

    // 阶段 A：同时转发音频 + 读取实时预览。
    loop {
        tokio::select! {
            chunk = audio.recv() => {
                match chunk {
                    Some(chunk) => {
                        if !chunk.is_empty() {
                            sink.send(Message::text(build_append(&chunk)))
                                .await
                                .context("send audio append")?;
                        }
                    }
                    None => break,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(t))) => {
                        let ev = parse_event(t.as_str());
                        if matches!(ev, AsrEvent::Finished) {
                            return Ok(());
                        }
                        let _ = events.send(ev);
                    }
                    Some(Ok(Message::Close(_))) => return Ok(()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        let _ = events.send(AsrEvent::Error(e.to_string()));
                        return Ok(());
                    }
                    None => break,
                }
            }
        }
    }

    if mode == Mode::Manual {
        sink.send(Message::text(build_commit())).await.context("send commit")?;
    }
    sink.send(Message::text(build_finish())).await.context("send finish")?;

    // 阶段 B：读取直到 session.finished / 关闭
    while let Some(msg) = stream.next().await {
        let msg = msg.context("ws message")?;
        match msg {
            Message::Text(t) => {
                let ev = parse_event(t.as_str());
                let done = matches!(ev, AsrEvent::Finished);
                let _ = events.send(ev);
                if done {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

/// 离线替身：无需网络，按固定脚本产生事件（供 `--fake` 与测试）。
pub fn fake_events(transcript: &str) -> Vec<AsrEvent> {
    vec![
        AsrEvent::SessionReady,
        AsrEvent::Preview(transcript.chars().take(2).collect()),
        AsrEvent::Preview(transcript.to_string()),
        AsrEvent::Final(transcript.to_string()),
        AsrEvent::Finished,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AsrConfig {
        AsrConfig { api_key: "sk-test".into(), ..Default::default() }
    }

    #[test]
    fn url_appends_model() {
        let c = AsrConfig { endpoint: "wss://h/api-ws/v1/realtime".into(), model: "m".into(), ..Default::default() };
        assert_eq!(c.url(), "wss://h/api-ws/v1/realtime?model=m");
        let c = AsrConfig { endpoint: "wss://h/x?a=1".into(), model: "m".into(), ..Default::default() };
        assert_eq!(c.url(), "wss://h/x?a=1&model=m");
    }

    #[test]
    fn manual_update_has_null_turn_detection() {
        let v: Value = serde_json::from_str(&build_session_update(&cfg(), Mode::Manual)).unwrap();
        assert_eq!(v["type"], "session.update");
        assert_eq!(v["session"]["input_audio_format"], "pcm");
        assert_eq!(v["session"]["sample_rate"], 16000);
        assert!(v["session"]["turn_detection"].is_null());
    }

    #[test]
    fn vad_update_has_server_vad_object() {
        let v: Value = serde_json::from_str(&build_session_update(&cfg(), Mode::Vad)).unwrap();
        assert_eq!(v["session"]["turn_detection"]["type"], "server_vad");
        assert!(v["session"]["turn_detection"]["silence_duration_ms"].is_number());
    }

    #[test]
    fn corpus_and_language_included_when_set() {
        let c = AsrConfig { language: Some("zh".into()), corpus: Some("WayVR OpenCode".into()), ..cfg() };
        let v: Value = serde_json::from_str(&build_session_update(&c, Mode::Manual)).unwrap();
        assert_eq!(v["session"]["input_audio_transcription"]["language"], "zh");
        assert_eq!(v["session"]["input_audio_transcription"]["corpus"]["text"], "WayVR OpenCode");
    }

    #[test]
    fn append_is_base64_json() {
        let v: Value = serde_json::from_str(&build_append(&[0u8, 1, 2, 3])).unwrap();
        assert_eq!(v["type"], "input_audio_buffer.append");
        assert_eq!(v["audio"], "AAECAw==");
    }

    #[test]
    fn parse_preview_combines_text_and_stash() {
        let ev = parse_event(r#"{"type":"conversation.item.input_audio_transcription.text","text":"今天天气","stash":"不错","emotion":"neutral"}"#);
        assert_eq!(ev, AsrEvent::Preview("今天天气不错".into()));
    }

    #[test]
    fn parse_completed_final() {
        let ev = parse_event(r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"今天天气不错。","emotion":"neutral"}"#);
        assert_eq!(ev, AsrEvent::Final("今天天气不错。".into()));
    }

    #[test]
    fn parse_ready_error_finished() {
        assert_eq!(parse_event(r#"{"type":"session.created"}"#), AsrEvent::SessionReady);
        assert_eq!(parse_event(r#"{"type":"session.updated"}"#), AsrEvent::SessionReady);
        assert_eq!(parse_event(r#"{"type":"error","error":{"message":"bad"}}"#), AsrEvent::Error("bad".into()));
        assert_eq!(parse_event(r#"{"type":"session.finished"}"#), AsrEvent::Finished);
        assert_eq!(parse_event("not json"), AsrEvent::Other);
    }
}

//! nekoya-stt — 语音识别 → 剪贴板 / VRChat OSC / OpenCode / WayVR 字幕面板。
//!
//! 设计要点（见 docs/architecture.md）：
//! - 只做语音识别，不解析语音指令。
//! - 只在内存保留「最近一次终稿」，不落盘、日志不含转写内容。
//! - 模块解耦：纯逻辑（config/state/wayvr_ipc::codec/engine/actions/hud）不依赖
//!   任何真实 IO，便于单元测试与 mock 替身。

pub mod actions;
pub mod asr;
pub mod audio;
pub mod config;
pub mod control;
pub mod daemon;
pub mod engine;
pub mod hud;
pub mod opencode;
pub mod sinks;
pub mod state;
pub mod tls;
pub mod wayvr_ipc;

/// 共享的运行时路径 / 环境变量名。
pub mod env {
    /// 千问 API Key 的环境变量名（唯一密钥来源，绝不写入仓库）。
    pub const DASHSCOPE_API_KEY: &str = "DASHSCOPE_API_KEY";
    /// VRChat OSC 端口覆盖。
    pub const VRC_OSC_PORT: &str = "NEKOYA_VRC_OSC_PORT";
    /// OpenCode 服务 URL 覆盖。
    pub const OPENCODE_URL: &str = "NEKOYA_OPENCODE_URL";
    /// OpenCode Basic 密码覆盖（默认从 service.json 读取）。
    pub const OPENCODE_PASSWORD: &str = "NEKOYA_OPENCODE_PASSWORD";
    /// 模型名覆盖。
    pub const MODEL: &str = "NEKOYA_MODEL";
    /// 是否启用 WayVR 联动（默认 false）。
    pub const WAYVR_ENABLED: &str = "NEKOYA_WAYVR_ENABLED";
}

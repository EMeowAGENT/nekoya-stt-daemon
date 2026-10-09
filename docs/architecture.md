# 架构

## 目标与边界

- **只做**：麦克风 → 千问云端实时 ASR → 文本动作。
- **不做**：语音指令解析、WayVR
  窗口控制（除字幕面板与显示）、执行外部程序（除自身 `act` 子命令）。
- 文本：只在内存保留**最近一次终稿**；**不落盘**；**日志不含转写内容**。

## 模块与依赖方向

```
        main / daemon (组装，唯一 new 具体实现的地方)
                     │
   ┌─────────────────┼───────────────────────────────┐
   ▼                 ▼                               ▼
engine(纯)      actions(纯)   hud(纯)   config/state(纯)   wayvr_ipc::codec(纯)
   状态机         按钮→动作     面板常量    配置/持久化          协议编解码
   └─────────────────┴───────────────────────────────┘
                     │  (后续阶段)
   ports(trait) ──► 真实适配器: PwRecordSource / QwenRealtime / WlCopySink /
                     VrcOscSink / HttpOpencode / WayvrIpcConn
                     │
                测试替身: FileSource / FakeAsr / MemorySink / CaptureSink /
                          MockHttp / MockWayvrIpc
```

规则：

- 纯模块不依赖任何 IO，可离线单测（当前 `cargo test` 31 项）。
- 具体类型只在组装层构造；模块间通过 trait/纯函数交互。
- `wayvr_ipc::codec` 为纯编解码，对金样本 JSON 断言。

## WayVR IPC（自实现，不依赖 WayVR 源码）

逆向自 wayvr 26.8.0 / 协议 v5：

- **socket**：Linux **抽象命名空间**，名字字节
  `\0/tmp/wayvr_ipc.sock`（`wayvr/src/ipc/ipc_server.rs` 用
  `to_ns_name::<GenericNamespaced>`；磁盘上没有该文件）。Rust 侧用
  `SocketAddr::from_abstract_name` + `UnixStream::connect_addr`。
- **分帧**：`u32 大端长度前缀 + UTF-8 JSON`，单包上限 128 KiB；`header+payload`
  必须**一次 write**。
- **握手**：首包
  `{"Handshake":{"protocol_version":5,"magic":"wayvr_ipc","client_name":"<1..31B>"}}`
  → `HandshakeSuccess{runtime}`；非法则 `Disconnect{reason}`。
- **枚举编码**：serde 默认外部标签；元组变体 → 数组（如
  `{"WlxOverlaySetVisible":["nekoya_stt",true]}`）。
- 本 daemon
  **只**使用：`WlxModifyPanel`（改文字/可见性）、`WlxOverlaySetVisible`（ensure-on，非
  toggle）。可选 `WlxWindowStateSet`（但按设计**不使用**，不碰位置）。
- 安全：该 IPC **无鉴权**（同机任何用户可连、甚至可 `WvrProcessLaunch`）——本
  daemon 绝不触达进程类命令。

## 千问 realtime（qwen3-asr-flash-realtime）

- 端点
  `wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=…`，`Authorization: Bearer $DASHSCOPE_API_KEY`。
- 音频：base64 JSON，PCM16/16k/mono，3200B≈100ms/块。
- 手动模式：`session.update` 置 `turn_detection: null`；说完发
  `input_audio_buffer.commit`。
- 自动(VAD)模式：`turn_detection:{type:"server_vad",…}`。
- 收尾：**必须** `session.finish` 并等
  `session.finished`，否则拿不到终稿。手动单会话建议 ≤60s（本 daemon 55s
  自动收尾）。
- 预览 = `text` + `stash`；终稿在
  `conversation.item.input_audio_transcription.completed.transcript`。
- 上下午文词表：`session.input_audio_transcription.corpus.text`（`lexicon.txt`，≤10000
  tokens，默认开）。
- 无 ITN、无时间戳、无 usage 回传。

## OpenCode（V2）

- 发现顺序：`$XDG_STATE_HOME/opencode/service.json`（含 `url`）→
  `$XDG_CONFIG_HOME/opencode/service.json`（`hostname` + `port`，默认
  **49374**）。**禁止静默拉起服务**。
- 鉴权：`Authorization: Basic base64("opencode:" + password)`。
- 会话：`GET /api/session`（根会话 `parentID=null`，按 `time.updated`
  倒序）；`POST /api/session/{id}/prompt {"text":…}`。
- 回复不在 VR 渲染（用户侧自行在 OpenCode TUI 查看）。

## 剪贴板限制（重要）

WayVR 有**两套隔离剪贴板**：宿主 Wayland 与内嵌 smithay 合成器。后者仅由 WayVR
进程内 `set_data_device_selection` 写入；客户端发起的 `set_selection` 被
`new_selection`（空实现）丢弃，且 IPC 无剪贴板包。→ 外部 daemon
**无法**写入内部剪贴板。本 daemon
因此**只写宿主剪贴板**（`wl-copy`）。这是刻意取舍，见 README「已知限制」。

## 数据与密钥

- 配置 `~/.config/nekoya-stt/config.toml`；状态 `state.toml`（仅选中的会话
  id）；词库 `lexicon.txt`。
- 密钥**仅** `DASHSCOPE_API_KEY`（systemd `EnvironmentFile`，仓库外，600）。

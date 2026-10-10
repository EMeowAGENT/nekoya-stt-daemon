# nekoya-stt-daemon

按住说话 → **千问云端实时语音识别** → 把**最近一次终稿**按需送到：宿主剪贴板 /
VRChat OSC / WayVR 虚拟键盘（Ctrl+V、回车）/ 选定的 **OpenCode** 会话；并在
WayVR 里显示实时字幕。

> 只做**语音识别**，**不解析语音指令**。daemon 不执行任何程序、不留存转写文本。

## 状态

| 阶段 | 内容                                                                                                        | 状态                               |
| ---- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| P0   | 骨架 + 纯逻辑模块（协议编解码 / 状态机 / 按钮动作 / 配置 / 状态 / OpenCode 端点发现）+ 单测 + `doctor` 自检 | ✅                                 |
| P1   | 音频采集 → 千问实时转写（transcribe，终端打印）                                                             | ✅（联网冒烟需 DASHSCOPE_API_KEY） |
| P2   | WayVR 字幕面板 + PTT 链路（`run` + 控制 socket + IPC，mock 集成测试）                                       | ✅                                 |
| P3   | 剪贴板 / OSC（sinks）+ 虚拟键盘动作（面板 `::SendKey`）                                                     | ✅                                 |
| P4   | OpenCode 会话列表 / 选择 / 推送（`act push`、手表长按）                                                     | ✅                                 |
| P5   | systemd 常驻、容错、词库                                                                                    | ⬜                                 |

## 架构

```
┌────────────── nekoya-stt（宿主 · systemd --user，绑 graphical-session）──────────────┐
│ pw-record ─► [PTT 门控] ─► [千问 realtime ASR] ─wss─► DashScope(北京)                 │
│ 16k/mono/s16le   手动 commit / VAD 自动       │ text+stash(预览) / transcript(终稿)   │
│                                               ▼                                      │
│                                   [只保留最近一次终稿，不落盘]                          │
│  面板按钮 ──`nekoya-stt act …`──► [Unix 控制 socket] ──┬─► wl-copy（宿主剪贴板）        │
│  手表长按 ──`nekoya-stt act auto`─►                     ├─► VRChat OSC(UDP 9000)        │
│                                                          ├─► HTTP Basic → OpenCode     │
│                                                          └─► IPC → WayVR 字幕面板       │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

解耦：核心逻辑只依赖 trait/纯函数，具体 IO（音频、ASR、剪贴板、OSC、HTTP、WayVR
IPC）在组装处注入 → 每个模块可独立测试、可用 mock 替身脱离
WayVR/OpenCode/麦克风。详见 `docs/architecture.md`。

## 构建

```sh
# 开发
cargo build --release
cargo test --all-targets

# 或走 PKGBUILD（只装 /usr/bin/nekoya-stt + LICENSE + systemd 用户单元）
makepkg -si
```

前置：本仓库**不依赖** `~/Wayvr` 任何文件——WayVR IPC
由本仓库自实现（`src/wayvr_ipc/`）。

> **Arch 打包注意**：`PKGBUILD` 已设 `options=('!lto')` 并在 `build()` 里清空
> `CFLAGS/CXXFLAGS/LDFLAGS`。原因是 makepkg 默认 `CFLAGS` 含 `-flto=auto`，会被
> `cc` 带进 `ring` 的 C 构建、产出 LTO 位码对象，导致最终链接报 `ring_core_*`
> 未定义。 手动 `cargo build --release`（不经过 makepkg）不受影响。

## 配置与密钥

- 配置：`~/.config/nekoya-stt/config.toml`（模板
  `config/config.toml.example`）。
- 密钥：**只**从环境变量 `DASHSCOPE_API_KEY` 读取。systemd 用户服务用
  `EnvironmentFile=%h/.config/nekoya-stt/env`（600 权限，仓库外）。模板见
  `env.example`。
- 其他覆盖：`NEKOYA_*`（见 `env.example`）。
- 自检：`nekoya-stt doctor`（检查密钥、WayVR IPC、OpenCode 端点可达性等）。

## WayVR 面板（手动，不安装）

1. 复制面板模板到主题目录：
   ```sh
   install -Dm644 wayvr/theme/gui/nekoya_stt.xml ~/.config/wayvr/theme/gui/nekoya_stt.xml
   ```
2. 启用面板（自定义 conf.d 片段）：
   ```sh
   install -Dm644 wayvr/conf.d/zz-nekoya.yaml ~/.config/wayvr/conf.d/zz-nekoya.yaml
   ```
3. 面板默认**隐藏**；daemon 只负责把文字写进 `transcript/status/session`
   与列表，绝不改动面板位置/朝向。

面板按钮：`0 PTT` · `1 复制到剪贴板` · `2 VRChat OSC` · `3 Ctrl+V` · `4 回车` ·
`5 推送 OpenCode` · `6 会话列表`。

在**手表**上加一个 STT 键（自行修改 `~/.config/wayvr/theme/gui/watch.xml`）：

```xml
<!-- 按下：开始录制；短按=切换主面板；长按=识别并推送 OpenCode -->
<Button id="btn_stt" macro="button_style"
  _press="::ShellExec /usr/bin/nekoya-stt act watch-down"
  _short_release="::ShellExec /usr/bin/nekoya-stt act watch-cancel"
  _short_release2="::OverlayToggle nekoya_stt"
  _long_release="::ShellExec /usr/bin/nekoya-stt act watch-up">
  <sprite width="32" height="32" src="/home/EMeowSystem/.config/wayvr/theme/assets/opencode-wordmark.svg" />
</Button>
```

行为：

- **按下**：开始录制（仅本地缓冲，**不开**识别连接）。
- 按住 **≥`ptt.start_after_ms`（默认 500）**：才建立识别会话，并把此前缓冲的音频补发
  （**不丢首字、不浪费连接**）。
- **短按**（松手时长 < WayVR 的 `long_press_duration`，默认 1.0s）：
  丢弃本次录音；面板显隐由 `::OverlayToggle nekoya_stt` 交给 WayVR 自己切换。
- **长按**（≥ `long_press_duration`）：收尾并把终稿**推送到 OpenCode 并发送**。

> 长短判定用 WayVR 的 `_short_release`/`_long_release`（阈值是全局 `long_press_duration`，
> 默认 1.0s；`long_press_time` 属性在当前版本无效）。daemon 另用 `ptt.start_after_ms`
> 决定"何时开流"——两者独立。

### 守护进程与控制通道

```sh
nekoya-stt run                 # 常驻；监听 $XDG_RUNTIME_DIR/nekoya-stt.sock
nekoya-stt status              # 读配置/状态
nekoya-stt ctl status          # 通过控制通道查询运行中的 daemon（无需 WayVR）
nekoya-stt ctl act list        # 拉取会话列表（填充面板）
nekoya-stt ctl act push        # 推送最近终稿到所选会话
nekoya-stt opencode-selftest   # 列根会话；可加 --session <id> --send <文本>
nekoya-stt wayvr-selftest      # 连 WayVR IPC 发一条面板文字（配合 mock 验证）
```

控制通道命令（面板/手表通过 `::ShellExec` 调用，也可手动）： `watch-down`
`watch-up` `ptt-down` `ptt-up` `show` `status` `act copy` `act osc` `act push`
`act list` `act select <n>` （`Ctrl+V`/`回车` 由面板自带的 `::SendKey`
完成，不经过本通道。）

`ctl` 常用：

```sh
nekoya-stt ctl watch-down      # 模拟手表按下（开始录制）
nekoya-stt ctl watch-up        # 松手（短按丢弃 / 长按推送）
nekoya-stt ctl act list        # 打开/关闭会话列表
nekoya-stt ctl act select 0    # 选择列表第 0 项并持久化到 state.toml
```

面板/手表按钮的等价入口（`act` 子命令，可直接手动调用/调试）：

```sh
nekoya-stt act watch-down   # = ctl watch-down
nekoya-stt act watch-up
nekoya-stt act copy         # = ctl act copy
nekoya-stt act osc
nekoya-stt act push
nekoya-stt act list
nekoya-stt act select 0
```

### 桌面预览面板（不开 WayVR）

WayVR 源码里的 `uidev` 可在普通窗口里渲染 wgui XML：

```sh
cd ~/Wayvr
TESTBED="$HOME/nekoya-stt-daemon/wayvr/theme/gui/nekoya_stt.xml" cargo run -p uidev
```

`F11` 打印控件树，用来核对元素 id。

## 测试（按模块）

```sh
scripts/test/run-all.sh          # 全部
scripts/test/01-codec.sh         # WayVR IPC 编解码（金样本）
scripts/test/02-engine-actions.sh# 状态机 + 按钮动作
scripts/test/03-opencode.sh      # OpenCode 端点发现 / Basic 鉴权
scripts/test/04-panel-xml.sh     # 面板 XML 元素 id 自检
scripts/test/05-doctor.sh        # 环境自检（离线安全）
```

需要 mock 服务的脚本（`06-wayvr-mock` / `07-osc` /
`08-clipboard`）在后续阶段补上。

## 已知限制（设计取舍）

- **剪贴板仅写宿主 Wayland**：WayVR
  内嵌合成器的剪贴板与宿主**互相隔离**，且外部进程**无法**写入（`new_selection`
  空实现、IPC 无剪贴板命令）。因此：
  - 「复制到剪贴板」(`1`) 与 `Ctrl+V`(`3`) 只在**宿主桌面**生效；
  - WayVR 串流窗口内的粘贴不生效；`OSC`(`2`)、`推送`(`5`)、`回车`(`4`)
    不受影响。
- **WayVR 联动默认禁用**：daemon 可在 WayVR 未运行时独立工作。
- 面板会话列表为固定 64 行槽（wgui 不能动态加元素），16 行可视、可滚动。

## 许可

MIT，见 `LICENSE`。

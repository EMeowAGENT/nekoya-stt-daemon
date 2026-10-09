#!/usr/bin/env bash
# 01 — WayVR IPC 编解码（金样本 JSON / 分帧 / 上限）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [01] wayvr_ipc codec =="
cargo test --lib wayvr_ipc "$@"

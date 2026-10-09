#!/usr/bin/env bash
# 05 — 环境自检（离线安全）：密钥/WayVR IPC/OpenCode 端点。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [05] doctor =="
cargo run --quiet -- doctor

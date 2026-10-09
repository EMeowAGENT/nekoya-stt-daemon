#!/usr/bin/env bash
# 09 — 千问 realtime 协议（session.update / append base64 / 事件解析）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [09] qwen asr protocol =="
cargo test --lib asr "$@"

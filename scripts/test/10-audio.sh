#!/usr/bin/env bash
# 10 — 音频分块与预滚缓冲（纯逻辑）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [10] audio chunking / preroll =="
cargo test --lib audio "$@"

#!/usr/bin/env bash
# 03 — OpenCode 端点发现（注册 vs 配置优先级）与 Basic 鉴权头形状。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [03] opencode discovery =="
cargo test --lib opencode "$@"

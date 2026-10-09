#!/usr/bin/env bash
# 02 — 识别状态机（单锁/最近终稿/预览）+ 按钮动作（144 截断/未选会话拒绝）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [02] engine + actions =="
cargo test --lib engine "$@"
cargo test --lib actions "$@"

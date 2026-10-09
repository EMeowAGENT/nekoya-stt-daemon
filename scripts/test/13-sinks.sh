#!/usr/bin/env bash
# 13 — 输出 sinks：OSC 报文编码（纯逻辑）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [13] sinks (osc) =="
cargo test --lib sinks "$@"

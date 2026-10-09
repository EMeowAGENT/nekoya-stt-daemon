#!/usr/bin/env bash
# 公共环境：切到仓库根，保证 cargo 可用（必要时用可写的 CARGO_HOME）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

# 若默认 CARGO_HOME 只读（如沙盒），允许外部通过 CARGO_HOME 覆盖。
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"

export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"

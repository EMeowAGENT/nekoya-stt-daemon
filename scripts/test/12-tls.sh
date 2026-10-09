#!/usr/bin/env bash
# 12 — rustls CryptoProvider 安装护栏（防止 WSS 握手 panic 回归）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [12] rustls provider =="
cargo test --lib tls "$@"

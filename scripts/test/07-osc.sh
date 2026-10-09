#!/usr/bin/env bash
# 07 — VRChat OSC：真实 UDP 收发（离线，无需 VRChat）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [07] osc (udp) =="
cargo test --test sinks_osc "$@"

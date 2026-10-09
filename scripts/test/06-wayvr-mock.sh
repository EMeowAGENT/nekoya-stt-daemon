#!/usr/bin/env bash
# 06 — WayVR IPC mock（TODO，P2/P3 实现）：
#   在抽象 socket \0/tmp/wayvr_ipc.sock 起一个假服务，断言 daemon 发来的
#   WlxModifyPanel SetText / WlxOverlaySetVisible。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [06] wayvr ipc mock =="
echo "TODO: 待 P2/P3 实现（src/bin/mock_wayvr_ipc 或 tests/support）"
exit 0

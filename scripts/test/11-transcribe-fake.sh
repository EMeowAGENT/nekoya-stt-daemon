#!/usr/bin/env bash
# 11 — 转写管线离线冒烟（--fake，不联网、不采集）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [11] transcribe --fake =="
out="$(cargo run --quiet -- transcribe --fake)"
echo "$out"
grep -q '^\[终稿\]' <<<"$out" || {
	echo "FAIL: 未产生终稿"
	exit 1
}
grep -q '^\[会话\] 结束' <<<"$out" || {
	echo "FAIL: 会话未结束"
	exit 1
}
echo "PASS"

#!/usr/bin/env bash
# 06 — WayVR IPC mock 集成：起假服务，验证 daemon 发送 WlxModifyPanel SetText 与
#      WlxOverlaySetVisible（ensure-on）。全程离线。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"

echo "== [06] wayvr ipc mock =="
cargo build --quiet

OUT="$(mktemp)"
./target/debug/mock_wayvr_ipc >"$OUT" 2>/dev/null &
MOCK=$!
trap 'kill "$MOCK" 2>/dev/null || true; wait "$MOCK" 2>/dev/null || true; rm -f "$OUT"' EXIT

# 若抽象 socket 已被真实 WayVR 占用，则无法用 mock 测试 → 跳过。
sleep 0.3
if ! kill -0 "$MOCK" 2>/dev/null; then
	echo "SKIP: 抽象 socket 已被占用（WayVR 正在运行？）"
	exit 0
fi

# 等待 mock 就绪：重试 selftest。
ok=0
for _ in $(seq 1 40); do
	if ./target/debug/nekoya-stt wayvr-selftest "你好，面板" >/dev/null 2>&1; then
		ok=1
		break
	fi
	sleep 0.1
done
[[ "$ok" -eq 1 ]] || {
	echo "FAIL: 无法连接 mock IPC"
	exit 1
}

sleep 0.2
echo "--- mock 收到的帧 ---"
cat "$OUT"

fail=0
grep -q '"WlxModifyPanel"' "$OUT" || {
	echo "FAIL: 未收到 WlxModifyPanel"
	fail=1
}
grep -q '"element":"transcript"' "$OUT" || {
	echo "FAIL: 未写入 transcript 元素"
	fail=1
}
grep -q '"SetText":"你好，面板"' "$OUT" || {
	echo "FAIL: SetText 内容不符"
	fail=1
}
grep -q '"WlxOverlaySetVisible":\["nekoya_stt",true\]' "$OUT" || {
	echo "FAIL: 未收到 ensure-on"
	fail=1
}

[[ "$fail" -eq 0 ]] && echo "PASS" || {
	echo "FAIL"
	exit 1
}

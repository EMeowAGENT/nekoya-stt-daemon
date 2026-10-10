#!/usr/bin/env bash
# 15 — 整机离线 E2E：mock WayVR IPC + mock OpenCode + daemon(run) + `ctl`。
#     验证控制通道、WayVR 面板写入（会话列表行）、OpenCode 会话拉取。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"

echo "== [15] daemon e2e (ctl + mock wayvr + mock opencode) =="
cargo build --quiet

TMP="$(mktemp -d)"
export XDG_RUNTIME_DIR="$TMP"
WV="$TMP/wv.out"
OC="$TMP/oc.out"
DM="$TMP/daemon.out"
PORT=49398

./target/debug/mock_wayvr_ipc >"$WV" 2>/dev/null &
MW=$!
./target/debug/mock_opencode "$PORT" >"$OC" 2>/dev/null &
MO=$!
cleanup() {
	kill "$MW" "$MO" "${DAEMON:-0}" 2>/dev/null || true
	wait 2>/dev/null || true
	rm -rf "$TMP"
}
trap cleanup EXIT

sleep 0.3
NEKOYA_WAYVR_ENABLED=1 NEKOYA_OPENCODE_URL="http://127.0.0.1:$PORT" \
	./target/debug/nekoya-stt run >"$DM" 2>&1 &
DAEMON=$!

# 等控制 socket 就绪
ok=0
for _ in $(seq 1 40); do
	if ./target/debug/nekoya-stt ctl status >/dev/null 2>&1; then
		ok=1
		break
	fi
	sleep 0.1
done
[[ "$ok" -eq 1 ]] || {
	echo "FAIL: daemon 控制通道未就绪"
	cat "$DM"
	exit 1
}

STATUS="$(./target/debug/nekoya-stt ctl status)"
echo "status: $STATUS"
LIST="$(./target/debug/nekoya-stt act list)"
echo "act list: $LIST"
ACT_STATUS="$(./target/debug/nekoya-stt act status)"
echo "act status: $ACT_STATUS"
sleep 0.3

echo "--- wayvr mock 收到 ---"
sed -n '1,6p' "$WV"
echo "--- opencode mock 收到 ---"
cat "$OC"

fail=0
grep -q '阶段=空闲' <<<"$STATUS" || {
	echo "FAIL: status 异常"
	fail=1
}
grep -q 'WayVR=已连接' <<<"$STATUS" || {
	echo "FAIL: WayVR 未连接"
	fail=1
}
grep -q 'ok: 2' <<<"$LIST" || {
	echo "FAIL: act list 未返回 2 条"
	fail=1
}
grep -q '阶段=空闲' <<<"$ACT_STATUS" || {
	echo "FAIL: act status 未走通"
	fail=1
}
grep -q 'LIST /api/session' "$OC" || {
	echo "FAIL: 未向 OpenCode 拉取会话"
	fail=1
}
grep -q '"session_row_0"' "$WV" || {
	echo "FAIL: 未填充会话列表行"
	fail=1
}
grep -q 'Mock Session 1' "$WV" || {
	echo "FAIL: 列表行文案不符"
	fail=1
}
grep -q '"element":"session_list","command":{"SetVisible":true}' "$WV" || {
	echo "FAIL: 会话列表未展开"
	fail=1
}

[[ "$fail" -eq 0 ]] && echo "PASS" || {
	echo "FAIL"
	exit 1
}

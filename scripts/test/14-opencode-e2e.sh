#!/usr/bin/env bash
# 14 — OpenCode 端到端（离线）：起假 OpenCode HTTP 服务，验证
#      opencode-selftest 能列会话、并能向指定会话发送 prompt。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"

echo "== [14] opencode e2e (mock http) =="
cargo build --quiet

PORT=49399
OUT="$(mktemp)"
./target/debug/mock_opencode "$PORT" >"$OUT" 2>/dev/null &
MOCK=$!
trap 'kill "$MOCK" 2>/dev/null || true; wait "$MOCK" 2>/dev/null || true; rm -f "$OUT"' EXIT

# 等端口就绪
ok=0
for _ in $(seq 1 40); do
	if NEKOYA_OPENCODE_URL="http://127.0.0.1:$PORT" ./target/debug/nekoya-stt opencode-selftest >/dev/null 2>&1; then
		ok=1
		break
	fi
	sleep 0.1
done
[[ "$ok" -eq 1 ]] || {
	echo "FAIL: mock OpenCode 未就绪"
	exit 1
}

RESP="$(NEKOYA_OPENCODE_URL="http://127.0.0.1:$PORT" ./target/debug/nekoya-stt opencode-selftest --session ses_mock1 --send '你好，OpenCode')"
echo "--- selftest 输出 ---"
echo "$RESP"

sleep 0.2
echo "--- mock 收到 ---"
cat "$OUT"

fail=0
grep -q 'ses_mock1' <<<"$RESP" || {
	echo "FAIL: 未列出会话"
	fail=1
}
grep -q '已发送 prompt 到 ses_mock1' <<<"$RESP" || {
	echo "FAIL: 未发送 prompt"
	fail=1
}
grep -q 'PROMPT /api/session/ses_mock1/prompt' "$OUT" || {
	echo "FAIL: mock 未收到 prompt 请求"
	fail=1
}
grep -q '你好，OpenCode' "$OUT" || {
	echo "FAIL: prompt 文本不符"
	fail=1
}

[[ "$fail" -eq 0 ]] && echo "PASS" || {
	echo "FAIL"
	exit 1
}

#!/usr/bin/env bash
# 04 — 面板 XML 自检：必需元素 id 与按钮绑定齐全（供 uidev 预览前检查）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"

XML="wayvr/theme/gui/nekoya_stt.xml"
echo "== [04] panel xml ($XML) =="
[[ -f "$XML" ]] || {
	echo "FAIL: $XML 不存在"
	exit 1
}

fail=0
need_ids=(transcript status session session_list btn_ptt btn_copy btn_osc btn_paste btn_enter btn_push btn_sessions btn_close_list)
for id in "${need_ids[@]}"; do
	if grep -q "id=\"$id\"" "$XML"; then
		echo "  ok   id=$id"
	else
		echo "  FAIL 缺少 id=$id"
		fail=1
	fi
done

# 64 行槽
rows=$(grep -c 'id="session_row_' "$XML" || true)
if [[ "$rows" -eq 64 ]]; then echo "  ok   64 行槽"; else
	echo "  FAIL 行槽数=$rows（期望 64）"
	fail=1
fi

# 绝对路径调用
if grep -q '::ShellExec /usr/bin/nekoya-stt' "$XML"; then echo "  ok   ShellExec 绝对路径"; else
	echo "  FAIL 未使用绝对路径"
	fail=1
fi
# 不得出现 toggle 面板显示（应用 ensure-on）
if grep -q '::OverlayToggle nekoya_stt' "$XML"; then
	echo "  FAIL 使用了 ::OverlayToggle（应为 IPC ensure-on）"
	fail=1
else echo "  ok   未用 OverlayToggle"; fi

[[ "$fail" -eq 0 ]] && echo "PASS" || {
	echo "FAIL"
	exit 1
}

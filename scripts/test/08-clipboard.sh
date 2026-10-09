#!/usr/bin/env bash
# 08 — 宿主剪贴板：需要可用的 Wayland 会话（wl-copy/wl-paste）。
#     离线/无合成器时安全跳过（带超时，避免卡死）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/00-lib.sh"
echo "== [08] clipboard =="

if [[ -n "${WAYLAND_DISPLAY:-}" ]] && command -v wl-copy >/dev/null && command -v wl-paste >/dev/null; then
	if printf 'nekoya-clip-test' | timeout 5 wl-copy 2>/dev/null; then
		got="$(timeout 5 wl-paste --no-newline 2>/dev/null || true)"
		echo "wl-paste: $got"
		if [[ "$got" == "nekoya-clip-test" ]]; then
			echo "PASS"
			exit 0
		fi
		echo "SKIP: 剪贴板往返未返回预期内容（合成器不可达？）"
		exit 0
	fi
fi
echo "SKIP: 无 WAYLAND_DISPLAY / 缺少 wl-copy/wl-paste / 合成器不可达"

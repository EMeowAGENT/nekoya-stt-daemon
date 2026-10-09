#!/usr/bin/env bash
# 跑全部模块测试脚本（每个脚本带 180s 超时护栏）。
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rc=0
for s in "$DIR"/[0-9][0-9]-*.sh; do
	base="$(basename "$s")"
	[[ "$base" == "00-lib.sh" ]] && continue
	echo
	echo "######################## $base ########################"
	if ! timeout 180 bash "$s"; then
		rc=1
		echo "!!! FAILED/TIMEOUT: $base"
	fi
done
echo
[[ "$rc" -eq 0 ]] && echo "ALL PASS" || {
	echo "SOME FAILED"
	exit 1
}

#!/usr/bin/env bash
# 跑全部模块测试脚本。
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rc=0
for s in "$DIR"/[0-9][0-9]-*.sh; do
	[[ "$(basename "$s")" == "00-lib.sh" ]] && continue
	echo
	echo "######################## $(basename "$s") ########################"
	if ! bash "$s"; then
		rc=1
		echo "!!! FAILED: $(basename "$s")"
	fi
done
echo
[[ "$rc" -eq 0 ]] && echo "ALL PASS" || {
	echo "SOME FAILED"
	exit 1
}

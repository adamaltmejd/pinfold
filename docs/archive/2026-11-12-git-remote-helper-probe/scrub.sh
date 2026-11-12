#!/bin/bash
# After run.sh and e2e.sh: replace the temp root (TMP), the system temp dir
# (SYSTMP) and the account name (user) in every log. Usage: scrub.sh TEMP_ROOT
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
root=$1
sys=$(cd "$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || echo /nonexistent)" 2>/dev/null && pwd -P || echo /nonexistent)
for f in "$HERE"/logs/*.log; do
	LC_ALL=C sed -i '' -e "s#$root#TMP#g" -e "s#/private$sys#SYSTMP#g" -e "s#$sys#SYSTMP#g" \
		-e "s#/var/folders/[^/]*/[^/]*/T#SYSTMP#g" -e "s#$(id -un)#user#g" "$f"
done

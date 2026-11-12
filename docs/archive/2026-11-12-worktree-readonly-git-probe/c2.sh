#!/bin/bash
# Case 2 driver. Usage: T=... c2.sh (R must not exist yet).
. "$(dirname "$0")/lib.sh"
D=$(dirname "$0"); CT=wtp2
fresh
box $CT
python3 "$D/c2.py" $CT 20 "$D/logs/c2.jsonl" >/dev/null
python3 - "$D/logs/c2.jsonl" <<'EOF'
import json, sys
rs = [json.loads(l) for l in open(sys.argv[1])]
print("trials", len(rs), "host_saw_edit", sum(r["host_saw_edit"] for r in rs),
      "host_commit_ok", sum(r["host_commit_rc"] == 0 for r in rs),
      "immediate_wrong", sum(not r["immediate_ok"] for r in rs),
      "later_wrong", sum(not r["later_ok"] for r in rs))
EOF
h $R/wt/x "git log --oneline | wc -l; git status --short; git fsck --no-progress"
container rm -f $CT >/dev/null

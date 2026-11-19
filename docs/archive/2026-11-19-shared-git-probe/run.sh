#!/bin/bash
# Usage: run.sh CONTAINER ROOT. The runs recorded in logs/, in order.
cd "$(dirname "$0")" || exit 1
export PFS_CT=$1 PFS_ROOT=$2
L=logs
python3 c1.py base 10 $L/c1-base.jsonl 0 0.5 1.0 2.0 >/dev/null
python3 c1.py status 10 $L/c1-status.jsonl 0 0.5 1.0 2.0 >/dev/null
python3 c1.py agent 10 $L/c1-agent.jsonl 0 0.5 1.0 2.0 >/dev/null
python3 c1.py waitvis 20 $L/c1-waitvis.jsonl 0 >/dev/null
python3 c1.py waitvis1 20 $L/c1-waitvis1.jsonl 0 >/dev/null
for m in none hostpoll starship nolocks ol0 status1 status5 log diffstat guestloop guestpoll; do
  python3 c4.py $m 3 30 $L/c4-$m.jsonl >/dev/null
done
echo done

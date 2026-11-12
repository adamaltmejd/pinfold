#!/bin/bash
# Three warm starts of each spec: up to ready, then down.
for spec in one three; do
  for i in 1 2 3; do
    out="$D/t-$spec-$i.out"
    printf '%s run %s: ' "$spec" "$i"
    "$D/up.sh" "$D/specs/$spec.json" "$out" | tail -1
    head -1 "$out" | grep -q '"event":"ready"' || { echo "not ready:"; cat "$out" "$out.err"; }
    "$P" box down "hl-$spec"
  done
done

#!/bin/bash
# Part B. usage: time.sh RUNS SPEC…; one warm-up of each spec, then RUNS
# interleaved starts of each: up to ready, then down. Expects P and D.
runs=$1; shift
once() {
  printf '%s %s: ' "$1" "$2"
  "$D/up.sh" "$D/$1.json" "/tmp/$1.out" > /tmp/up.txt
  grep -q '"event":"ready"' /tmp/up.txt || { echo "not ready:"; cat /tmp/up.txt "/tmp/$1.out.err"; }
  tail -1 /tmp/up.txt
  "$D/down.sh" "$1"
}
for s in "$@"; do once "$s" warm-up; done
for i in $(seq "$runs"); do for s in "$@"; do once "$s" "$i"; done; done

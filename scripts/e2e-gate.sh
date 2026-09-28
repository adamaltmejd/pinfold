#!/bin/sh
# Yard's e2e gate: the Linux suite on GitHub's runners (e2e-linux.sh) and
# the macOS suite on this Mac, at the same time. Yard runs host gates one
# after another, so one gate running both takes the longer suite's time,
# not the sum. Each suite's output is printed whole, after both finish.
set -u

h=$(eval echo "~$(id -un)")
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# A host gate gets a fresh HOME. git and gh need the real one; cargo needs
# only the real rustup and cargo homes.
HOME="$h" sh scripts/e2e-linux.sh >"$out/linux" 2>&1 &
linux_pid=$!
RUSTUP_HOME="$h/.rustup" CARGO_HOME="$h/.cargo" cargo test -p e2e --locked >"$out/macos" 2>&1
macos=$?
wait "$linux_pid"
linux=$?

echo "=== e2e-macos: exit $macos"
cat "$out/macos"
echo "=== e2e-linux: exit $linux"
cat "$out/linux"
[ "$macos" -eq 0 ] && [ "$linux" -eq 0 ]

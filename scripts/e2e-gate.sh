#!/bin/sh
# Run Linux CI on GitHub and the macOS suite on this Mac in parallel.
# Print each suite's output after both finish.
set -u

h=$(eval echo "~$(id -un)")
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# Git and gh need the user's HOME; cargo needs its toolchain homes.
HOME="$h" sh scripts/e2e-linux.sh >"$out/linux" 2>&1 &
linux_pid=$!
RUSTUP_HOME="$h/.local/share/rustup" CARGO_HOME="$h/.cargo" cargo test -p e2e --locked >"$out/macos" 2>&1
macos=$?
wait "$linux_pid"
linux=$?

echo "=== e2e-macos: exit $macos"
cat "$out/macos"
echo "=== e2e-linux: exit $linux"
cat "$out/linux"
[ "$macos" -eq 0 ] && [ "$linux" -eq 0 ]

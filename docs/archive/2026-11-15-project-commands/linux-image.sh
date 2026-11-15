#!/bin/bash
set -eu
W=/tmp/pfimage
mkdir -p "$W/c" "$W/s/pinfold" "$W/empty"
touch "$W/s/pinfold/maintenance"
export XDG_CONFIG_HOME=$W/c XDG_STATE_HOME=$W/s PINFOLD_NO_UPDATE_CHECK=1
cat > "$W/Containerfile" <<'RECIPE'
FROM debian:13-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
RUN git config --system user.name user && git config --system user.email user@example.invalid
RECIPE
cd ~/pinfold
df -h /
target/x86_64-unknown-linux-musl/debug/pinfold image build probe-commands --containerfile "$W/Containerfile" --context "$W/empty"

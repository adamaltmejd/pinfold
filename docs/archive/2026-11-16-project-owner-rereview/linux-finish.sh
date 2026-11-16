set -euo pipefail
cd ~/pinfold
cargo fmt --check
RUSTC_WRAPPER= cargo clippy --all-targets --locked --target x86_64-unknown-linux-musl -- -D warnings
RUSTC_WRAPPER= cargo build --locked --target x86_64-unknown-linux-musl
sudo apt-get -o DPkg::Lock::Timeout=600 install -y gdb curl ca-certificates e2fsprogs
sudo chgrp "$(id -gn)" /dev/kvm
sudo chmod 660 /dev/kvm
mkdir -p /tmp/pfrr/image /tmp/pfrr/istate/pinfold /tmp/pfrr/iconfig
: > /tmp/pfrr/istate/pinfold/maintenance
cat > /tmp/pfrr/image/Containerfile <<'CF'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends git curl perl ca-certificates && rm -rf /var/lib/apt/lists/*
CF
export XDG_CONFIG_HOME=/tmp/pfrr/iconfig XDG_STATE_HOME=/tmp/pfrr/istate PINFOLD_NO_UPDATE_CHECK=1
B=target/x86_64-unknown-linux-musl/debug/pinfold
"$B" image build probe-owner-rereview --containerfile /tmp/pfrr/image/Containerfile --context /tmp/pfrr/image --allow deb.debian.org --allow security.debian.org
D=docs/archive/2026-11-16-project-owner-rereview
I=localhost/pinfold/image-probe-owner-rereview:latest
python3 "$D/status.py" "$B" "$I" > /tmp/pfrr/final-status.log 2>&1
python3 "$D/crossing-output.py" "$B" "$I" > /tmp/pfrr/final-crossing.log 2>&1
echo FRESH-CONTROLS-PASS

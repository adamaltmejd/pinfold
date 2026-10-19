#!/bin/sh
# Build pinfold (musl, release) from the copied tree in ~/pinfold.
set -eu
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none >/dev/null
. "$HOME/.cargo/env"
cd ~/pinfold
rustup show active-toolchain >/dev/null 2>&1 || rustup toolchain install
rustup target add x86_64-unknown-linux-musl
export CC_x86_64_unknown_linux_musl=musl-gcc
cargo build --release --locked --target x86_64-unknown-linux-musl -p pinfold 2>&1 | tail -3
ls -l target/x86_64-unknown-linux-musl/release/pinfold

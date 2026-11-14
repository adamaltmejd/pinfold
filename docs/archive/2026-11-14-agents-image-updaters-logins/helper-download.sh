#!/bin/sh
set -eu
# Fixture program expects this scratch directory. No host login is used.
mkdir -p /tmp/pfh.298
curl -fL https://github.com/openai/codex/releases/download/rust-v0.167.1/codex-app-server-aarch64-apple-darwin.tar.gz -o /tmp/pfh.298/helper.tar.gz
printf '%s  %s\n' d8ccd88f64b776bbf8d330a46a0d2d80eae7580dcd981e0f97070eae58a4bf63 /tmp/pfh.298/helper.tar.gz | shasum -a 256 -c -
tar -xzf /tmp/pfh.298/helper.tar.gz -C /tmp/pfh.298
python3 "$(dirname "$0")/helper-fixture.py"

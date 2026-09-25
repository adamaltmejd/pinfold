#!/bin/sh
# Update the ADD --checksum pins in profile/Containerfile to the latest
# bun, rtk and ponytail releases. Run from anywhere; needs curl and
# sha256sum (Linux) or shasum (macOS).
set -eu

dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
containerfile="$dir/Containerfile"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The sha256 of stdin.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi | cut -d' ' -f1
}

# Download one release asset and print its sha256.
pin() {
    curl -fsSL "$1" | sha256
}

github_tag() {
    curl -fsSL "$1" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1
}

bun=$(github_tag https://api.github.com/repos/oven-sh/bun/releases/latest)
rtk=$(github_tag https://api.github.com/repos/rtk-ai/rtk/releases/latest)
ponytail=$(curl -fsSL https://registry.npmjs.org/@dietrichgebert/ponytail \
    | grep -o '"dist-tags":{[^}]*}' \
    | sed -n 's/.*"latest":"\([^"]*\)".*/\1/p')
test -n "$bun" && test -n "$rtk" && test -n "$ponytail"

bun_aarch64="https://github.com/oven-sh/bun/releases/download/$bun/bun-linux-aarch64.zip"
bun_x64="https://github.com/oven-sh/bun/releases/download/$bun/bun-linux-x64.zip"
rtk_aarch64="https://github.com/rtk-ai/rtk/releases/download/$rtk/rtk-aarch64-unknown-linux-gnu.tar.gz"
rtk_x64="https://github.com/rtk-ai/rtk/releases/download/$rtk/rtk-x86_64-unknown-linux-musl.tar.gz"
ponytail_tgz="https://registry.npmjs.org/@dietrichgebert/ponytail/-/ponytail-$ponytail.tgz"

{
    printf 'ADD --checksum=sha256:%s %s /tmp/bun-aarch64.zip\n' "$(pin "$bun_aarch64")" "$bun_aarch64"
    printf 'ADD --checksum=sha256:%s %s /tmp/bun-x64.zip\n' "$(pin "$bun_x64")" "$bun_x64"
    printf 'ADD --checksum=sha256:%s %s /tmp/rtk-aarch64.tar.gz\n' "$(pin "$rtk_aarch64")" "$rtk_aarch64"
    printf 'ADD --checksum=sha256:%s %s /tmp/rtk-x64.tar.gz\n' "$(pin "$rtk_x64")" "$rtk_x64"
    printf 'ADD --checksum=sha256:%s %s /tmp/ponytail.tgz\n' "$(pin "$ponytail_tgz")" "$ponytail_tgz"
} > "$tmp/pins"

awk -v pins="$tmp/pins" '
    /^# >>> pins$/ { print; while ((getline line < pins) > 0) print line; close(pins); skip=1; next }
    /^# <<< pins$/ { print; skip=0; next }
    !skip { print }
' "$containerfile" > "$containerfile.new"
mv "$containerfile.new" "$containerfile"
echo "updated $containerfile: bun $bun, rtk $rtk, ponytail $ponytail"

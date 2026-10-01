#!/bin/sh
# Update every pin to its latest upstream release: the ADD --checksum pins
# in profile/Containerfile and the harness rows in
# crates/pinfold/harnesses.toml. Run from anywhere; needs curl, python3, awk/sed,
# and sha256sum (Linux) or shasum (macOS).
set -eu

dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname -- "$dir")
containerfile="$root/profile/Containerfile"
harnessfile="$root/crates/pinfold/harnesses.toml"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp" "$containerfile.new" "$harnessfile.new"' EXIT

# Download $1 and print its sha256. The download lands in a file first so a
# failed curl is fatal instead of hashing an empty stream.
pin() {
    curl -fsSL -o "$tmp/asset" "$1"
    if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi \
        < "$tmp/asset" | cut -d' ' -f1
}

gh_json='Accept: application/vnd.github+json'

# Top-level string $2 in JSON $1.
json_string() {
    python3 - "$1" "$2" <<'PY'
import json
import sys

with open(sys.argv[1]) as source:
    print(json.load(source).get(sys.argv[2], ""))
PY
}

# Asset $2's $3 ("browser_download_url" or "digest") in the GitHub release
# JSON $1; empty when the release has no such asset.
github_asset() {
    python3 - "$1" "$2" "$3" <<'PY'
import json
import sys

with open(sys.argv[1]) as source:
    for asset in json.load(source)["assets"]:
        if asset["name"] == sys.argv[2]:
            print(asset.get(sys.argv[3]) or "")
            break
PY
}

# --- profile pins --------------------------------------------------------

curl -fsSL -H "$gh_json" -o "$tmp/bun.json" https://api.github.com/repos/oven-sh/bun/releases/latest
bun=$(json_string "$tmp/bun.json" tag_name)
curl -fsSL -H "$gh_json" -o "$tmp/rtk.json" https://api.github.com/repos/rtk-ai/rtk/releases/latest
rtk=$(json_string "$tmp/rtk.json" tag_name)
curl -fsSL -o "$tmp/ponytail.json" https://registry.npmjs.org/@dietrichgebert/ponytail
ponytail=$(grep -o '"dist-tags":{[^}]*}' "$tmp/ponytail.json" \
    | sed -n 's/.*"latest":"\([^"]*\)".*/\1/p')
if [ -z "$bun" ] || [ -z "$rtk" ] || [ -z "$ponytail" ]; then
    echo "bump-pins: could not read the latest bun, rtk or ponytail release" >&2
    exit 1
fi

bun_aarch64="https://github.com/oven-sh/bun/releases/download/$bun/bun-linux-aarch64.zip"
bun_x64="https://github.com/oven-sh/bun/releases/download/$bun/bun-linux-x64.zip"
rtk_aarch64="https://github.com/rtk-ai/rtk/releases/download/$rtk/rtk-aarch64-unknown-linux-gnu.tar.gz"
rtk_x64="https://github.com/rtk-ai/rtk/releases/download/$rtk/rtk-x86_64-unknown-linux-musl.tar.gz"
ponytail_tgz="https://registry.npmjs.org/@dietrichgebert/ponytail/-/ponytail-$ponytail.tgz"

bun_aarch64_sha=$(pin "$bun_aarch64")
bun_x64_sha=$(pin "$bun_x64")
rtk_aarch64_sha=$(pin "$rtk_aarch64")
rtk_x64_sha=$(pin "$rtk_x64")
ponytail_sha=$(pin "$ponytail_tgz")

{
    printf 'ADD --checksum=sha256:%s %s /tmp/bun-aarch64.zip\n' "$bun_aarch64_sha" "$bun_aarch64"
    printf 'ADD --checksum=sha256:%s %s /tmp/bun-x64.zip\n' "$bun_x64_sha" "$bun_x64"
    printf 'ADD --checksum=sha256:%s %s /tmp/rtk-aarch64.tar.gz\n' "$rtk_aarch64_sha" "$rtk_aarch64"
    printf 'ADD --checksum=sha256:%s %s /tmp/rtk-x64.tar.gz\n' "$rtk_x64_sha" "$rtk_x64"
    printf 'ADD --checksum=sha256:%s %s /tmp/ponytail.tgz\n' "$ponytail_sha" "$ponytail_tgz"
} > "$tmp/pins"

# --- harness pins --------------------------------------------------------

# The current file as "<kind>\t<harness>\t<value>" rows: each harness's
# version and each asset's url.
awk -F'"' '
    /^\[\[harness\]\]$/ { name = ""; next }
    /^name = / && name == "" { name = $2; next }
    /^version = / && name != "" { print "version\t" name "\t" $2; next }
    /^url = / && name != "" { print "url\t" name "\t" $2 }
' "$harnessfile" > "$tmp/current"

curl -fsSL -H "$gh_json" -o "$tmp/pi.json" https://api.github.com/repos/earendil-works/pi/releases/latest
pi_tag=$(json_string "$tmp/pi.json" tag_name)
curl -fsSL -H "$gh_json" -o "$tmp/codex.json" https://api.github.com/repos/openai/codex/releases/latest
codex_tag=$(json_string "$tmp/codex.json" tag_name)
claude_version=$(curl -fsSL https://downloads.claude.ai/claude-code-releases/latest \
    | sed 's/[[:space:]]//g')

case $pi_tag in
v[0-9]*) pi_version=${pi_tag#v} ;;
*)
    echo "bump-pins: pi's latest tag ${pi_tag:-?} is not v<version>" >&2
    exit 1
    ;;
esac
case $codex_tag in
rust-v[0-9]*) codex_version=${codex_tag#rust-v} ;;
*)
    echo "bump-pins: codex's latest tag ${codex_tag:-?} is not rust-v<version>" >&2
    exit 1
    ;;
esac
case $codex_version in
*-*)
    echo "bump-pins: codex's latest release $codex_tag is not stable" >&2
    exit 1
    ;;
esac
if [ -z "$claude_version" ]; then
    echo "bump-pins: could not read claude's latest version" >&2
    exit 1
fi
curl -fsSL -o "$tmp/claude-manifest.json" \
    "https://downloads.claude.ai/claude-code-releases/$claude_version/manifest.json"

: > "$tmp/updates"
: > "$tmp/report"
for harness in $(sed -n 's/^name = "\([^"]*\)"/\1/p' "$harnessfile"); do
    old=$(awk -F'\t' -v h="$harness" '$1 == "version" && $2 == h { print $3 }' "$tmp/current")
    case $harness in
    pi) version=$pi_version; json="$tmp/pi.json" ;;
    claude) version=$claude_version; json="$tmp/claude-manifest.json" ;;
    codex) version=$codex_version; json="$tmp/codex.json" ;;
    *)
        echo "bump-pins: no source to update harness $harness" >&2
        exit 1
        ;;
    esac
    if [ -z "$old" ]; then
        echo "bump-pins: cannot read $harness's pinned version" >&2
        exit 1
    fi

    printf 'version\t%s\t%s\n' "$harness" "$version" >> "$tmp/updates"
    printf '%s %s -> %s\n' "$harness" "$old" "$version" >> "$tmp/report"

    awk -F'\t' -v h="$harness" '$1 == "url" && $2 == h { print $3 }' "$tmp/current" > "$tmp/urls"
    while IFS= read -r oldurl; do
        if [ "$harness" = claude ]; then
            os_arch=$(printf '%s\n' "$oldurl" \
                | sed -n 's|.*/claude-code-releases/[^/]*/\([^/]*\)/claude$|\1|p')
            publisher=$(awk -F'"' -v want="$os_arch" '
                /^  "platforms": \{/ { platforms = 1; next }
                platforms && $2 == want { found = 1; next }
                platforms && found && $2 == "checksum" { print $4; exit }
            ' "$json")
            newurl="https://downloads.claude.ai/claude-code-releases/$version/$os_arch/claude"
        else
            asset=${oldurl##*/}
            publisher=$(github_asset "$json" "$asset" digest)
            newurl=$(github_asset "$json" "$asset" browser_download_url)
        fi
        if [ -z "$publisher" ] || [ -z "$newurl" ]; then
            echo "bump-pins: $harness has no release asset for $oldurl" >&2
            exit 1
        fi
        got=$(pin "$newurl")
        if [ "$got" != "${publisher#sha256:}" ]; then
            echo "bump-pins: $harness $newurl: sha256 $got does not match the publisher's" >&2
            exit 1
        fi
        printf 'url\t%s\t%s\n' "$oldurl" "$newurl" >> "$tmp/updates"
        printf 'sha\t%s\t%s\n' "$oldurl" "$got" >> "$tmp/updates"
    done < "$tmp/urls"
done

# --- write ---------------------------------------------------------------

awk -v pins="$tmp/pins" '
    /^# >>> pins$/ { print; while ((getline line < pins) > 0) print line; close(pins); skip=1; next }
    /^# <<< pins$/ { print; skip=0; next }
    !skip { print }
' "$containerfile" > "$containerfile.new"

awk -F'\t' '
    NR == FNR {
        if ($1 == "version") ver[$2] = $3
        else if ($1 == "url") newurl[$2] = $3
        else if ($1 == "sha") sha[$2] = $3
        next
    }
    /^\[\[harness\]\]$/ { harness = ""; print; next }
    /^name = "/ && harness == "" {
        value = $0
        sub(/^name = "/, "", value); sub(/".*$/, "", value)
        harness = value
        print
        next
    }
    /^version = "/ && harness in ver { print "version = \"" ver[harness] "\""; next }
    /^url = "/ {
        value = $0
        sub(/^url = "/, "", value); sub(/".*$/, "", value)
        if (value in newurl) { print "url = \"" newurl[value] "\""; last = value }
        else { print; last = "" }
        next
    }
    /^sha256 = "/ {
        if (last != "" && last in sha) print "sha256 = \"" sha[last] "\""
        else print
        next
    }
    { print }
' "$tmp/updates" "$harnessfile" > "$harnessfile.new"

mv "$containerfile.new" "$containerfile"
mv "$harnessfile.new" "$harnessfile"

cat "$tmp/report"
echo "updated $containerfile: bun $bun, rtk $rtk, ponytail $ponytail"

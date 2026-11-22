FROM debian:trixie-slim AS base

RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get upgrade -y; \
    apt-get install -y --no-install-recommends \
        ca-certificates curl git ripgrep fd-find jq less procps; \
    ln -s /usr/bin/fdfind /usr/local/bin/fd; \
    git config --system user.name pi; \
    git config --system user.email pi@pinfold.invalid; \
    rm -rf /var/lib/apt/lists/*

# Box work runs as the host uid. Name the usual ones pinfold, with the
# project box's home /home/pinfold, where Debian leaves the uid and name
# free: 501:20 (macOS) and 1000:1000 (Linux). useradd refuses a second
# account of one name, so uid 1000's line is appended; a lookup by name
# finds uid 501. The prompt omits \u, so any other uid gets no
# "I have no name!".
RUN set -eu; \
    if ! getent passwd pinfold >/dev/null && ! getent group pinfold >/dev/null; then \
        if ! getent passwd 501 >/dev/null; then \
            useradd -M -N -u 501 -g 20 -d /home/pinfold -s /bin/bash pinfold; \
        fi; \
        if ! getent group 1000 >/dev/null; then \
            groupadd -g 1000 pinfold; \
        fi; \
        if ! getent passwd 1000 >/dev/null; then \
            printf '%s\n' 'pinfold:x:1000:1000::/home/pinfold:/bin/bash' >> /etc/passwd; \
        fi; \
    fi; \
    printf '%s\n' "PS1='pinfold:\\w\\\$ '" >> /etc/bash.bashrc

# An interactive shell in a project box that starts in the home, such as a
# Rex pane, moves to the project. Every interactive shell marks the project
# trusted in ~/.claude.json, since the VM is the trust boundary; a
# symlink, an unparseable file or one jq reads as empty is left alone. A Claude Code rewriting the file at the
# same moment can drop the flag; the next shell restores it.
RUN printf '%s\n' \
    'case $- in *i*)' \
    '    if [ -n "${PINFOLD_PROJECT-}" ] && [ -d "$PINFOLD_PROJECT" ]; then' \
    '        [ "$PWD" = "$HOME" ] && cd "$PINFOLD_PROJECT"' \
    '        (' \
    '            f="$HOME/.claude.json"; p=".projects[\$project].hasTrustDialogAccepted"' \
    '            [ -L "$f" ] && exit' \
    '            jq -e --arg project "$PINFOLD_PROJECT" "$p == true" "$f" >/dev/null 2>&1 && exit' \
    '            t=$(mktemp "$f.XXXXXX" 2>/dev/null) || exit' \
    '            if [ -e "$f" ]; then' \
    '                jq --arg project "$PINFOLD_PROJECT" "$p = true" "$f"' \
    '            else' \
    '                jq -n --arg project "$PINFOLD_PROJECT" "{} | $p = true"' \
    '            fi >"$t" 2>/dev/null && [ -s "$t" ] && mv -f "$t" "$f" || rm -f "$t"' \
    '        )' \
    '    fi' \
    '    ;;' \
    'esac' >> /etc/bash.bashrc

# Agents are resolved and installed only here; profiles built on this
# image inherit them. Each build passes a new PINFOLD_AGENT_RESOLVE, so the
# resolver reruns and the install reruns only when a version changed.
# GitHub releases resolve through github.com's latest-release redirect and
# each release's SHA256SUMS, not api.github.com, whose unauthenticated rate
# limit fails builds from shared addresses (#385).
FROM base AS agent-resolve
ARG TARGETARCH
ARG PINFOLD_AGENT_RESOLVE=default
RUN set -eu; \
    : "$PINFOLD_AGENT_RESOLVE"; \
    case "${TARGETARCH}" in \
        arm64) arch=arm64; target=aarch64-unknown-linux-musl ;; \
        amd64) arch=x64; target=x86_64-unknown-linux-musl ;; \
        *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    github_latest() { \
        location=$(curl -fsS -o /dev/null -w '%{redirect_url}' -H 'Cache-Control: no-cache' "https://github.com/$1/releases/latest"); \
        printf '%s\n' "${location#"https://github.com/$1/releases/tag/"}" | grep -Ex "$2"; \
    }; \
    github_asset() { \
        curl -fsSL "https://github.com/$1/releases/download/$2/$3" -o /tmp/SHA256SUMS; \
        jq -nRec --arg name "$4" --arg url "https://github.com/$1/releases/download/$2/$4" \
            '[inputs | capture("^(?<sha256>[a-f0-9]{64})  (?<name>.+)$") | select(.name == $name)] | select(length == 1) | {url: $url, sha256: .[0].sha256}' /tmp/SHA256SUMS; \
    }; \
    echo "resolving stable agent releases"; \
    pi_version=$(github_latest earendil-works/pi 'v[0-9]+\.[0-9]+\.[0-9]+'); \
    pi_asset=$(github_asset earendil-works/pi "$pi_version" SHA256SUMS "pi-linux-$arch.tar.gz"); \
    claude_base=https://downloads.claude.ai/claude-code-releases; \
    claude_version=$(curl -fsSL -H 'Cache-Control: no-cache' "$claude_base/stable"); \
    printf '%s\n' "$claude_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; \
    curl -fsSL "$claude_base/$claude_version/manifest.json" -o /tmp/claude-manifest.json; \
    claude_checksum=$(jq -er --arg platform "linux-$arch" '.platforms[$platform].checksum | strings | select(test("^[a-f0-9]{64}$"))' /tmp/claude-manifest.json); \
    codex_version=$(github_latest openai/codex 'rust-v[0-9]+\.[0-9]+\.[0-9]+'); \
    codex_asset=$(github_asset openai/codex "$codex_version" codex-package_SHA256SUMS "codex-package-$target.tar.gz"); \
    jq -nSc --arg pi "$pi_version" --argjson pi_asset "$pi_asset" \
        --arg claude "$claude_version" --arg claude_url "$claude_base/$claude_version/linux-$arch/claude" --arg claude_sha256 "$claude_checksum" \
        --arg codex "$codex_version" --argjson codex_asset "$codex_asset" \
        '{pi: ($pi_asset + {version: $pi}), claude: {version: $claude, url: $claude_url, sha256: $claude_sha256}, codex: ($codex_asset + {version: $codex})}' > /resolved.json; \
    rm -f /tmp/SHA256SUMS /tmp/claude-manifest.json

FROM base AS agents
ARG TARGETARCH
COPY --from=agent-resolve /resolved.json /resolved.json
RUN set -eu; \
    case "${TARGETARCH}" in \
        arm64) target=aarch64-unknown-linux-musl ;; \
        amd64) target=x86_64-unknown-linux-musl ;; \
        *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    mkdir -p /opt/pinfold/agents; \
    downloads=$(mktemp -d); \
    receipt=/opt/pinfold/agents/install.txt; \
    : > "$receipt"; \
    download() { \
        agent=$1; file=$2; \
        url=$(jq -er --arg agent "$agent" '.[$agent].url' /resolved.json); \
        checksum=$(jq -er --arg agent "$agent" '.[$agent].sha256' /resolved.json); \
        printf '%s\n' "$checksum" | grep -Eq '^[a-f0-9]{64}$'; \
        echo "agent download: $url" | tee -a "$receipt"; \
        curl -fsSL "$url" -o "$file"; \
        printf '%s  %s\n' "$checksum" "$file" | sha256sum -c -; \
    }; \
    pi_version=$(jq -er '.pi.version' /resolved.json); \
    claude_version=$(jq -er '.claude.version' /resolved.json); \
    codex_version=$(jq -er '.codex.version' /resolved.json); \
    pi_archive="$downloads/pi.tar.gz"; \
    download pi "$pi_archive"; \
    tar -xzf "$pi_archive" -C /opt/pinfold/agents; \
    test -x /opt/pinfold/agents/pi/pi; \
    test -f /opt/pinfold/agents/pi/package.json; \
    test -f /opt/pinfold/agents/pi/photon_rs_bg.wasm; \
    for asset in theme assets native export-html docs examples; do test -d "/opt/pinfold/agents/pi/$asset"; done; \
    claude_binary="$downloads/claude"; \
    download claude "$claude_binary"; \
    mkdir -p /opt/pinfold/agents/claude; \
    install -m 0755 "$claude_binary" /opt/pinfold/agents/claude/claude; \
    codex_archive="$downloads/codex.tar.gz"; \
    download codex "$codex_archive"; \
    mkdir -p /opt/pinfold/agents/codex; \
    tar -xzf "$codex_archive" -C /opt/pinfold/agents/codex; \
    rm -rf "$downloads"; \
    jq -e --arg version "${codex_version#rust-v}" --arg target "$target" '.layoutVersion == 1 and .version == $version and .target == $target and .variant == "codex" and .entrypoint == "bin/codex" and .execServer == "bin/codex-exec-server" and .resourcesDir == "codex-resources" and .pathDir == "codex-path"' /opt/pinfold/agents/codex/codex-package.json; \
    for binary in codex codex-code-mode-host codex-exec-server; do test -x "/opt/pinfold/agents/codex/bin/$binary"; done; \
    test -x /opt/pinfold/agents/codex/codex-path/rg; \
    test -x /opt/pinfold/agents/codex/codex-resources/bwrap; \
    rm -f /resolved.json; \
    chmod -R a+rX /opt/pinfold/agents; \
    printf 'installed agents: pi=%s claude=%s codex=%s\n' "$pi_version" "$claude_version" "$codex_version" | tee -a "$receipt"; \
    ln -s /opt/pinfold/agents/pi/pi /usr/local/bin/pi; \
    ln -s /opt/pinfold/agents/claude/claude /usr/local/bin/claude; \
    ln -s /opt/pinfold/agents/codex/bin/codex /usr/local/bin/codex; \
    ln -s /opt/pinfold/agents/codex/bin/codex-code-mode-host /usr/local/bin/codex-code-mode-host
ENV PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_UPDATES=1

# Every built-in recipe ends here, so this link's own installs are stripped too.
RUN find / -xdev -perm /6000 -type f -exec chmod a-s {} +

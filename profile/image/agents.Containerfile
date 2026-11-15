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
    github_asset() { \
        jq -ec --arg name "$2" --arg url "$3" \
            '.assets | map(select(.name == $name)) | select(length == 1) | .[0] | {url: .browser_download_url, sha256: (.digest | strings | select(test("^sha256:[a-f0-9]{64}$")) | ltrimstr("sha256:"))} | select(.url == $url)' "$1"; \
    }; \
    echo "resolving stable agent releases"; \
    curl -fsSL -H 'Cache-Control: no-cache' https://api.github.com/repos/earendil-works/pi/releases/latest -o /tmp/pi-release.json; \
    pi_version=$(jq -er 'select(.draft == false and .prerelease == false) | .tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))' /tmp/pi-release.json); \
    pi_asset=$(github_asset /tmp/pi-release.json "pi-linux-$arch.tar.gz" "https://github.com/earendil-works/pi/releases/download/$pi_version/pi-linux-$arch.tar.gz"); \
    claude_base=https://downloads.claude.ai/claude-code-releases; \
    claude_version=$(curl -fsSL -H 'Cache-Control: no-cache' "$claude_base/stable"); \
    printf '%s\n' "$claude_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; \
    curl -fsSL "$claude_base/$claude_version/manifest.json" -o /tmp/claude-manifest.json; \
    claude_checksum=$(jq -er --arg platform "linux-$arch" '.platforms[$platform].checksum | strings | select(test("^[a-f0-9]{64}$"))' /tmp/claude-manifest.json); \
    curl -fsSL -H 'Cache-Control: no-cache' https://api.github.com/repos/openai/codex/releases/latest -o /tmp/codex-release.json; \
    codex_version=$(jq -er 'select(.draft == false and .prerelease == false) | .tag_name | select(test("^rust-v[0-9]+\\.[0-9]+\\.[0-9]+$"))' /tmp/codex-release.json); \
    codex_asset=$(github_asset /tmp/codex-release.json "codex-package-$target.tar.gz" "https://github.com/openai/codex/releases/download/$codex_version/codex-package-$target.tar.gz"); \
    jq -nSc --arg pi "$pi_version" --argjson pi_asset "$pi_asset" \
        --arg claude "$claude_version" --arg claude_url "$claude_base/$claude_version/linux-$arch/claude" --arg claude_sha256 "$claude_checksum" \
        --arg codex "$codex_version" --argjson codex_asset "$codex_asset" \
        '{pi: ($pi_asset + {version: $pi}), claude: {version: $claude, url: $claude_url, sha256: $claude_sha256}, codex: ($codex_asset + {version: $codex})}' > /resolved.json; \
    rm -f /tmp/pi-release.json /tmp/claude-manifest.json /tmp/codex-release.json

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
    rm -f "$pi_archive"; \
    test -x /opt/pinfold/agents/pi/pi; \
    test -f /opt/pinfold/agents/pi/package.json; \
    test -f /opt/pinfold/agents/pi/photon_rs_bg.wasm; \
    for asset in theme assets native export-html docs examples; do test -d "/opt/pinfold/agents/pi/$asset"; done; \
    claude_binary="$downloads/claude"; \
    download claude "$claude_binary"; \
    mkdir -p /opt/pinfold/agents/claude; \
    install -m 0755 "$claude_binary" /opt/pinfold/agents/claude/claude; \
    rm -f "$claude_binary"; \
    codex_archive="$downloads/codex.tar.gz"; \
    download codex "$codex_archive"; \
    mkdir -p /opt/pinfold/agents/codex; \
    tar -xzf "$codex_archive" -C /opt/pinfold/agents/codex; \
    rm -f "$codex_archive"; \
    rmdir "$downloads"; \
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

FROM base AS agents
ARG TARGETARCH
RUN set -eu; \
    case "${TARGETARCH}" in \
        arm64) arch=arm64; target=aarch64-unknown-linux-musl ;; \
        amd64) arch=x64; target=x86_64-unknown-linux-musl ;; \
        *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    mkdir -p /opt/pinfold/agents; \
    downloads=$(mktemp -d); \
    receipt=/opt/pinfold/agents/install.txt; \
    date -u '+resolved_at_utc=%Y-%m-%dT%H:%M:%SZ' > "$receipt"; \
    download() { \
        url=$1; file=$2; checksum=$3; \
        printf '%s\n' "$checksum" | grep -Eq '^[a-f0-9]{64}$'; \
        echo "agent download: $url" | tee -a "$receipt"; \
        curl -fsSL "$url" -o "$file"; \
        printf '%s  %s\n' "$checksum" "$file" | sha256sum -c -; \
    }; \
    github_asset() { \
        metadata=$1; name=$2; file=$3; \
        url=$(jq -er --arg name "$name" '.assets | map(select(.name == $name)) | select(length == 1) | .[0].browser_download_url' "$metadata"); \
        checksum=$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .digest | strings | select(startswith("sha256:")) | ltrimstr("sha256:")' "$metadata"); \
        download "$url" "$file" "$checksum"; \
    }; \
    echo "resolving stable agent releases"; \
    curl -fsSL -H 'Cache-Control: no-cache' https://api.github.com/repos/earendil-works/pi/releases/latest -o /tmp/pi-release.json; \
    pi_version=$(jq -er 'select(.draft == false and .prerelease == false) | .tag_name | select(test("^v[0-9]+\\.[0-9]+\\.[0-9]+$"))' /tmp/pi-release.json); \
    pi_archive="$downloads/pi.tar.gz"; \
    github_asset /tmp/pi-release.json "pi-linux-$arch.tar.gz" "$pi_archive"; \
    tar -xzf "$pi_archive" -C /opt/pinfold/agents; \
    rm -f "$pi_archive"; \
    test -x /opt/pinfold/agents/pi/pi; \
    test -f /opt/pinfold/agents/pi/package.json; \
    test -f /opt/pinfold/agents/pi/photon_rs_bg.wasm; \
    for asset in theme assets native export-html docs examples; do test -d "/opt/pinfold/agents/pi/$asset"; done; \
    claude_base=https://downloads.claude.ai/claude-code-releases; \
    claude_version=$(curl -fsSL -H 'Cache-Control: no-cache' "$claude_base/stable"); \
    printf '%s\n' "$claude_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; \
    curl -fsSL "$claude_base/$claude_version/manifest.json" -o /tmp/claude-manifest.json; \
    claude_checksum=$(jq -er --arg platform "linux-$arch" '.platforms[$platform].checksum' /tmp/claude-manifest.json); \
    claude_binary="$downloads/claude"; \
    download "$claude_base/$claude_version/linux-$arch/claude" "$claude_binary" "$claude_checksum"; \
    mkdir -p /opt/pinfold/agents/claude; \
    install -m 0755 "$claude_binary" /opt/pinfold/agents/claude/claude; \
    rm -f "$claude_binary"; \
    curl -fsSL -H 'Cache-Control: no-cache' https://api.github.com/repos/openai/codex/releases/latest -o /tmp/codex-release.json; \
    codex_version=$(jq -er 'select(.draft == false and .prerelease == false) | .tag_name | select(test("^rust-v[0-9]+\\.[0-9]+\\.[0-9]+$"))' /tmp/codex-release.json); \
    codex_archive="$downloads/codex.tar.gz"; \
    github_asset /tmp/codex-release.json "codex-package-$target.tar.gz" "$codex_archive"; \
    mkdir -p /opt/pinfold/agents/codex; \
    tar -xzf "$codex_archive" -C /opt/pinfold/agents/codex; \
    rm -f "$codex_archive"; \
    rmdir "$downloads"; \
    jq -e --arg version "${codex_version#rust-v}" --arg target "$target" '.layoutVersion == 1 and .version == $version and .target == $target and .variant == "codex" and .entrypoint == "bin/codex" and .execServer == "bin/codex-exec-server" and .resourcesDir == "codex-resources" and .pathDir == "codex-path"' /opt/pinfold/agents/codex/codex-package.json; \
    for binary in codex codex-code-mode-host codex-exec-server; do test -x "/opt/pinfold/agents/codex/bin/$binary"; done; \
    test -x /opt/pinfold/agents/codex/codex-path/rg; \
    test -x /opt/pinfold/agents/codex/codex-resources/bwrap; \
    rm -f /tmp/pi-release.json /tmp/claude-manifest.json /tmp/codex-release.json; \
    chmod -R a+rX /opt/pinfold/agents; \
    printf 'installed agents: pi=%s claude=%s codex=%s\n' "$pi_version" "$claude_version" "$codex_version" | tee -a "$receipt"; \
    ln -s /opt/pinfold/agents/pi/pi /usr/local/bin/pi; \
    ln -s /opt/pinfold/agents/claude/claude /usr/local/bin/claude; \
    ln -s /opt/pinfold/agents/codex/bin/codex /usr/local/bin/codex; \
    ln -s /opt/pinfold/agents/codex/bin/codex-code-mode-host /usr/local/bin/codex-code-mode-host
ENV PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1 \
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 DISABLE_UPDATES=1


FROM base AS rtk-arm64
ADD --checksum=sha256:8d6d1aad9e69b42481eda7039507d1f7ee93698f87713cecd873d287c1931632 https://github.com/rtk-ai/rtk/releases/download/v0.51.0/rtk-aarch64-unknown-linux-gnu.tar.gz /tmp/rtk-aarch64.tar.gz

FROM base AS rtk-amd64
ADD --checksum=sha256:5028d3b19a8f0990d30fec9fbb07e32782bc5698e618fb1861aad8a9ccba4eb5 https://github.com/rtk-ai/rtk/releases/download/v0.51.0/rtk-x86_64-unknown-linux-musl.tar.gz /tmp/rtk-x64.tar.gz

FROM rtk-${TARGETARCH} AS rtk-install
RUN set -eux; \
    tar -xzf /tmp/rtk-*.tar.gz -C /tmp; \
    install -m 0755 /tmp/rtk /usr/local/bin/rtk; \
    export HOME=/tmp/rtk-home; \
    mkdir -p "${HOME}" /opt/pinfold/packages; \
    rtk init -g --agent pi; \
    mv "${HOME}/.pi/agent/extensions/rtk.ts" /opt/pinfold/packages/rtk.ts; \
    chmod a+r /opt/pinfold/packages/rtk.ts

FROM base AS ponytail-install
ADD --checksum=sha256:b5a24dcf8ec158326f3d3da92d7c0ea522dfbdf6c3de62a6a2ab98d5e683895f https://registry.npmjs.org/@dietrichgebert/ponytail/-/ponytail-4.12.0.tgz /tmp/ponytail.tgz
RUN set -eux; \
    mkdir -p /opt/pinfold/packages/ponytail; \
    tar -xzf /tmp/ponytail.tgz --strip-components=1 -C /opt/pinfold/packages/ponytail; \
    chmod -R a+rX /opt/pinfold/packages/ponytail

FROM documents AS full
COPY --from=rtk-install /usr/local/bin/rtk /usr/local/bin/rtk
COPY --from=rtk-install /opt/pinfold/packages/rtk.ts /opt/pinfold/packages/rtk.ts
COPY --from=ponytail-install /opt/pinfold/packages/ponytail /opt/pinfold/packages/ponytail
# gh from GitHub's signed apt repository.
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    mkdir -p /etc/apt/keyrings; \
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /etc/apt/keyrings/githubcli-archive-keyring.gpg; \
    chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg; \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list; \
    apt-get update; \
    apt-get install -y --no-install-recommends gh; \
    rm -rf /var/lib/apt/lists/*

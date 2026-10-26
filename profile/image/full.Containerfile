
FROM base AS rtk-arm64
ADD --checksum=sha256:cd627c4091ad90b5e5aca83ca5af66ee4b8aa8537a736533fa90544f461f1c2b https://github.com/rtk-ai/rtk/releases/download/v1.0.1/rtk-aarch64-unknown-linux-gnu.tar.gz /tmp/rtk-aarch64.tar.gz

FROM base AS rtk-amd64
ADD --checksum=sha256:5b919c3bccb799b67a61866d0e617410a4fcdf173e2614440e59de2fa2003436 https://github.com/rtk-ai/rtk/releases/download/v1.0.1/rtk-x86_64-unknown-linux-musl.tar.gz /tmp/rtk-x64.tar.gz

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
ADD --checksum=sha256:d2e8ef2f98e8f70bf40ffcd3ff157634fe13b2572e10ad70a62fc26db6de6dc5 https://registry.npmjs.org/@dietrichgebert/ponytail/-/ponytail-5.3.2.tgz /tmp/ponytail.tgz
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

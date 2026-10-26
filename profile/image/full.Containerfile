
FROM base AS fff-arm64
ADD --checksum=sha256:702bd8fe5d861fb5016a4fa0e62545a0a3e802dcc6b7232a3d02e54f0f90187e https://registry.npmjs.org/@ff-labs/fff-bin-linux-arm64-gnu/-/fff-bin-linux-arm64-gnu-0.11.0.tgz /tmp/fff-bin-aarch64.tgz

FROM base AS fff-amd64
ADD --checksum=sha256:36c5f09b7e315e4e1870fc2311dfae392d0f6e6197f3c0b0697e15d8d8344ebf https://registry.npmjs.org/@ff-labs/fff-bin-linux-x64-gnu/-/fff-bin-linux-x64-gnu-0.11.0.tgz /tmp/fff-bin-x64.tgz

FROM fff-${TARGETARCH} AS fff-install
ARG TARGETARCH
ADD --checksum=sha256:d25cacb52974a8c99be1ecf88a0ba554459402a8850798cc722e08b088d21de3 https://registry.npmjs.org/@ff-labs/pi-fff/-/pi-fff-0.11.0.tgz /tmp/pi-fff.tgz
ADD --checksum=sha256:516e6e3ff779664b455b08207d03a80b97e25d9123a579b67f6f151fda358c73 https://registry.npmjs.org/@ff-labs/fff-bun/-/fff-bun-0.11.0.tgz /tmp/fff-bun.tgz
# fff-node, pi-fff's Node fallback, is deliberately not staged: pi runs extensions under Bun.
RUN set -eux; \
    case "${TARGETARCH}" in \
        arm64) native=fff-bin-linux-arm64-gnu ;; \
        amd64) native=fff-bin-linux-x64-gnu ;; \
        *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    fff=/opt/pinfold/packages/pi-fff; \
    mkdir -p "${fff}/node_modules/@ff-labs/fff-bun" "${fff}/node_modules/@ff-labs/${native}"; \
    tar -xzf /tmp/pi-fff.tgz --strip-components=1 -C "${fff}"; \
    tar -xzf /tmp/fff-bun.tgz --strip-components=1 -C "${fff}/node_modules/@ff-labs/fff-bun"; \
    tar -xzf /tmp/fff-bin-*.tgz --strip-components=1 -C "${fff}/node_modules/@ff-labs/${native}"; \
    chmod -R a+rX "${fff}"

FROM base AS ponytail-install
ADD --checksum=sha256:d2e8ef2f98e8f70bf40ffcd3ff157634fe13b2572e10ad70a62fc26db6de6dc5 https://registry.npmjs.org/@dietrichgebert/ponytail/-/ponytail-5.3.2.tgz /tmp/ponytail.tgz
RUN set -eux; \
    mkdir -p /opt/pinfold/packages/ponytail; \
    tar -xzf /tmp/ponytail.tgz --strip-components=1 -C /opt/pinfold/packages/ponytail; \
    chmod -R a+rX /opt/pinfold/packages/ponytail

FROM documents AS full
COPY --from=fff-install /opt/pinfold/packages/pi-fff /opt/pinfold/packages/pi-fff
COPY --from=ponytail-install /opt/pinfold/packages/ponytail /opt/pinfold/packages/ponytail
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

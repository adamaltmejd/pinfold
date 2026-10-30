
# Only the selected architecture's stage is built. Release archives stay
# in build stages; final images copy the installed files.
FROM base AS bun-arm64
ADD --checksum=sha256:baa6bb36f51d6e58561e02bbea825038a9df454f673d6d776e8bbf86f8c7850e https://github.com/oven-sh/bun/releases/download/bun-v1.4.4/bun-linux-aarch64.zip /tmp/bun-aarch64.zip

FROM base AS bun-amd64
ADD --checksum=sha256:bf8ccbfb2b0b80159e0f9a6d15c55317add8077534dbc6130a3edb52fccb395c https://github.com/oven-sh/bun/releases/download/bun-v1.4.4/bun-linux-x64.zip /tmp/bun-x64.zip

FROM bun-${TARGETARCH} AS bun-install
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    unzip -q /tmp/bun-*.zip -d /tmp/bun; \
    install -m 0755 /tmp/bun/bun-linux-*/bun /usr/local/bin/bun

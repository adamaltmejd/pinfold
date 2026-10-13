
# Only the selected architecture's stage is built. Release archives stay
# in build stages; final images copy the installed files.
FROM base AS bun-arm64
ADD --checksum=sha256:efa9813da5ed72423bf847f916e8d2c47c0d776add972354026a75e10da9aa21 https://github.com/oven-sh/bun/releases/download/bun-v1.4.3/bun-linux-aarch64.zip /tmp/bun-aarch64.zip

FROM base AS bun-amd64
ADD --checksum=sha256:3e730528d89775f03f87f05c06787c4df4099d0a37f3f252e7b829cb23816e18 https://github.com/oven-sh/bun/releases/download/bun-v1.4.3/bun-linux-x64.zip /tmp/bun-x64.zip

FROM bun-${TARGETARCH} AS bun-install
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    unzip -q /tmp/bun-*.zip -d /tmp/bun; \
    install -m 0755 /tmp/bun/bun-linux-*/bun /usr/local/bin/bun

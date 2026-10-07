
# Only the selected architecture's stage is built. Release archives stay
# in build stages; final images copy the installed files.
FROM base AS bun-arm64
ADD --checksum=sha256:54328bbc2d9c8e0c9f892c544d66c57a83b84139e34909e5ee81758f1ac8fda7 https://github.com/oven-sh/bun/releases/download/bun-v1.4.2/bun-linux-aarch64.zip /tmp/bun-aarch64.zip

FROM base AS bun-amd64
ADD --checksum=sha256:36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913 https://github.com/oven-sh/bun/releases/download/bun-v1.4.2/bun-linux-x64.zip /tmp/bun-x64.zip

FROM bun-${TARGETARCH} AS bun-install
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    unzip -q /tmp/bun-*.zip -d /tmp/bun; \
    install -m 0755 /tmp/bun/bun-linux-*/bun /usr/local/bin/bun

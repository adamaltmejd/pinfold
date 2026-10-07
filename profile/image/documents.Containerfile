
FROM base AS anydoc-arm64
ADD --checksum=sha256:ed71661425a9959009d39ba495dbc565cc27a27f4eb0c93336b89f217e5c56b2 https://registry.npmjs.org/@firecrawl/anydoc-linux-arm64-gnu/-/anydoc-linux-arm64-gnu-0.2.4.tgz /tmp/anydoc-aarch64.tgz

FROM base AS anydoc-amd64
ADD --checksum=sha256:ca822ea3ad29a9b9ca6b7f6d2a6b3d5311153af25a6c35a7f5bdf0791fe7f2c9 https://registry.npmjs.org/@firecrawl/anydoc-linux-x64-gnu/-/anydoc-linux-x64-gnu-0.2.4.tgz /tmp/anydoc-x64.tgz

FROM anydoc-${TARGETARCH} AS anydoc-install
ARG TARGETARCH
ADD --checksum=sha256:625bf6cdc24cc91eee8fbfe084c1fa56c71b17f034341d4a23bc8df0fdee31bd https://registry.npmjs.org/@firecrawl/anydoc/-/anydoc-0.2.4.tgz /tmp/anydoc.tgz
RUN set -eux; \
    case "${TARGETARCH}" in \
        arm64) native=anydoc-linux-arm64-gnu ;; \
        amd64) native=anydoc-linux-x64-gnu ;; \
        *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    anydoc=/opt/pinfold/anydoc/node_modules/@firecrawl; \
    mkdir -p "${anydoc}/anydoc" "${anydoc}/${native}"; \
    tar -xzf /tmp/anydoc.tgz --strip-components=1 -C "${anydoc}/anydoc"; \
    tar -xzf /tmp/anydoc-*.tgz --strip-components=1 -C "${anydoc}/${native}"

FROM base AS documents
COPY --from=bun-install /usr/local/bin/bun /usr/local/bin/bun
COPY --from=anydoc-install /opt/pinfold/anydoc /opt/pinfold/anydoc
RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get install -y --no-install-recommends poppler-utils; \
    rm -rf /var/lib/apt/lists/*; \
    printf '#!/bin/sh\nexec /usr/local/bin/bun /opt/pinfold/anydoc/node_modules/@firecrawl/anydoc/cli.js "$@"\n' > /usr/local/bin/anydoc; \
    chmod 0755 /usr/local/bin/anydoc; \
    chmod -R a+rX /opt/pinfold/anydoc

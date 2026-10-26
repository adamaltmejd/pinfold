ARG TARGETARCH
FROM debian:trixie-slim AS base

RUN set -eux; \
    export DEBIAN_FRONTEND=noninteractive; \
    apt-get update; \
    apt-get upgrade -y; \
    apt-get install -y --no-install-recommends \
        ca-certificates curl git ripgrep fd-find jq less; \
    ln -s /usr/bin/fdfind /usr/local/bin/fd; \
    git config --system user.name pi; \
    git config --system user.email pi@pinfold.invalid; \
    rm -rf /var/lib/apt/lists/*

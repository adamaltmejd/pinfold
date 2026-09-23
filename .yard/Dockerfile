# The image Yard builds lane workers and gates from ([isolation].dockerfile in
# .yard/config.toml). Lanes read it from the candidate's history, so a change
# reaches them only once committed. Yard needs /bin/sh, /usr/bin/env, sleep and
# git in it.
#
# The base floats with its tag, as pinfold's own profile image does; the Rust
# toolchain is pinned to match rust-toolchain.toml.
FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates git curl xz-utils gcc libc6-dev \
       ripgrep fd-find jq less \
    && ln -s /usr/bin/fdfind /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# Rust from the standalone installer into /usr/local, not rustup: a gate runs
# under env -i with HOME=/tmp, where rustup's proxies find no toolchain.
# Keep RUST_VERSION in step with rust-toolchain.toml.
ARG RUST_VERSION=1.98.1
RUN set -eu; \
    triple="$(uname -m)-unknown-linux-gnu"; \
    name="rust-${RUST_VERSION}-${triple}"; \
    cd /tmp; \
    curl -fsSLO "https://static.rust-lang.org/dist/${name}.tar.xz"; \
    curl -fsSLO "https://static.rust-lang.org/dist/${name}.tar.xz.sha256"; \
    sha256sum -c "${name}.tar.xz.sha256"; \
    tar -xJf "${name}.tar.xz"; \
    "./${name}/install.sh" --prefix=/usr/local --disable-ldconfig \
        --components="rustc,cargo,rust-std-${triple},rustfmt-preview,clippy-preview"; \
    rm -rf /tmp/rust-*; \
    cargo --version; cargo fmt --version; cargo clippy --version

# trufflehog for the secrets gate, pinned by version and checksum. Gates have
# no network, so it runs with --no-verification: every match counts.
ARG TRUFFLEHOG_VERSION=3.97.8
RUN set -eu; \
    case "$(uname -m)" in \
        aarch64) arch=arm64; sum=5145332e23582ccda55651b18162e5b88f21411e65385a0df8d82753654208b7 ;; \
        x86_64) arch=amd64; sum=8f71019bcb8c6c44242b6e34f68c3755f8f9c8496d3063b7a8b5626fb9bff220 ;; \
        *) echo "no trufflehog pin for $(uname -m)" >&2; exit 1 ;; \
    esac; \
    file="trufflehog_${TRUFFLEHOG_VERSION}_linux_${arch}.tar.gz"; \
    curl -fsSLo "/tmp/${file}" "https://github.com/trufflesecurity/trufflehog/releases/download/v${TRUFFLEHOG_VERSION}/${file}"; \
    echo "${sum}  /tmp/${file}" | sha256sum -c; \
    tar -xzf "/tmp/${file}" -C /usr/local/bin trufflehog; \
    rm "/tmp/${file}"; \
    trufflehog --version

# Gates have no network, so the workspace's crates are fetched here into a
# fixed CARGO_HOME that the gate commands name. The build context is the
# candidate's own tree, so a dependency a candidate adds is fetched with it.
# Writable by any uid: cargo takes a lock file in CARGO_HOME even offline.
COPY Cargo.toml Cargo.lock /tmp/fetch/
COPY crates /tmp/fetch/crates
RUN set -eu; \
    cd /tmp/fetch; \
    CARGO_HOME=/usr/local/cargo cargo fetch --locked; \
    chmod -R a+rwX /usr/local/cargo; \
    rm -rf /tmp/fetch

WORKDIR /workspace

# The toolchain rust-toolchain.toml pins, with its components. Boxes set
# their own HOME and PATH, so the toolchain is copied out of rustup and linked
# onto the default PATH; rust-toolchain.toml is not read inside a box.
FROM docker.io/library/rust:1.99.0-slim-trixie AS rust
RUN rustup component add rustfmt clippy && cp -a "$(rustc --print sysroot)" /toolchain

FROM pinfold/profile-default:latest
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rust /toolchain /opt/rust
RUN ln -s /opt/rust/bin/* /usr/local/bin/ && cargo --version && cargo clippy --version

# trufflehog for the secrets gate, pinned by version and checksum. Gates have
# no network, so it runs with --no-verification: every match counts.
ARG TRUFFLEHOG_VERSION=3.98.0
RUN set -eu; \
    case "$(uname -m)" in \
        aarch64) arch=arm64; sum=49d38516c6a5ec4633fc31502b79978a01bbafdf892381e511d4bd6e74865563 ;; \
        x86_64) arch=amd64; sum=05920d08aa23c233d410eefe5c73ae9db86dc7e766b64d6a9368678912daeda7 ;; \
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

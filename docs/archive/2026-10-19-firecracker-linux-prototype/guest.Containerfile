# One layer over pinfold's profile image: the vsock relays, the exec agent
# and an init. Built with `podman build --build-arg PROFILE=...`.
ARG PROFILE=pinfold/profile-default:latest
FROM ${PROFILE}
USER root
RUN set -eux; export DEBIAN_FRONTEND=noninteractive; apt-get update; \
    apt-get install -y --no-install-recommends socat procps iproute2; \
    rm -rf /var/lib/apt/lists/*; \
    mkdir -p /workspace /home/box; chown 1000:1000 /home/box
COPY pf-agent /sbin/pf-agent
COPY pf-init pf-shell /sbin/
# harden.Containerfile again: no setuid files.
RUN find / -xdev -perm /6000 -type f -exec chmod a-s {} +

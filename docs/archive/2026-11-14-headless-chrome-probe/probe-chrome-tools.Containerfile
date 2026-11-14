FROM localhost/pinfold/profile-probe-chrome:latest
RUN mkdir -p /opt/probe-chrome-tools; \
    (set -eux; export DEBIAN_FRONTEND=noninteractive; \
    apt-get -o Debug::Acquire::http=true update; \
    apt-get -o Debug::Acquire::http=true install -y --no-install-recommends strace python3; \
    rm -rf /var/lib/apt/lists/*; \
    find / -xdev -perm /6000 -type f -exec chmod a-s {} +) > /opt/probe-chrome-tools/build.log 2>&1

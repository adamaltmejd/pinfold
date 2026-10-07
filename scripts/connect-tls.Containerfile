FROM pinfold/profile-default:latest

# The slow CONNECT fixture needs a real TLS client inside the box.
RUN apt-get update && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/* \
    && find / -xdev -type f -perm /6000 -exec chmod a-s {} +

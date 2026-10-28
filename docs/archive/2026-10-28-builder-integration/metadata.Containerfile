FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends acl attr libcap2-bin \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir /fixtures \
    && printf 'owned-fixture\n' >/fixtures/owned \
    && chown 123:456 /fixtures/owned && chmod 751 /fixtures/owned \
    && ln /fixtures/owned /fixtures/hard \
    && ln -s /fixtures/owned /fixtures/sym \
    && cp /bin/true /fixtures/cap \
    && setcap cap_net_bind_service=ep /fixtures/cap \
    && setfacl -m u:345:r--,u:1000:r-- /fixtures/owned \
    && setfattr -n user.phase6b -v yes /fixtures/owned \
    && touch /fixtures/gone
RUN rm /fixtures/gone

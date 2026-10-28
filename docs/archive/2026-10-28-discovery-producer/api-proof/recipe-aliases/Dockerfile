ARG BASE=localhost/pinfold/image-zeta:latest
FROM example.invalid/unreachable AS unused
RUN false
FROM $BASE AS first
FROM localhost/pinfold/image-alpha:latest
COPY --from=first /fixture-marker /copied

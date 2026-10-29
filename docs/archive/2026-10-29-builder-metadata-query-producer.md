# Native metadata query producer, 2026-10-29

This producer-only change replaces the helper's preloaded OCI metadata stores
with bounded config queries. It does not adopt the consumer runtime change or
change its bootstrap pin. The working public consumer remains on bootstrap
`09393a02ccfa5c05cee4689e3792e2ec240677d9269a1cbbc4b247f697acffc4`.
The new helper requires a matching future consumer and must not be substituted
under that existing artifact identity.

The helper still runs the exact pinned native Dockerfile conversion frontend.
Alias-to-store/manifest options remain native named contexts. For an OCI config
request it writes one newline-delimited JSON query containing the store and
digest-qualified reference. Its stdin receives a big-endian u32 length and exact
config bytes, bounded by the existing 1 MiB JSON limit. It returns the requested
manifest digest and unchanged config bytes. It caches the exact store,
reference and session binding. Direct source metadata requests may replay only
that served binding; they cannot open another host lookup or use removed stores.

Local and external resolutions retain the existing shared 4096-item/1 MiB
receipt bounds. The helper writes only external resolutions to its required
exclusive output file. The future host must independently record each local
consultation before replying, retain the existing input budget and refuse
further lookup before RUN. Cancellation closes helper stdio so pending metadata
I/O can end. No new dependency, parser, registry or credential source is added.

BuildKit remains 0.34.1 at
`f5b59a1f2fb25a0cb7072c398cab9aed7835da4e`, using its committed vendor tree.
Go remains the producer's pinned 1.26.10. No producer pin or artifact bytes were
changed. The existing factory must build this exact committed helper before
any provenance, publication or consumer-pin decision. No output artifact hash
is predicted here.

gofmt and source review passed. The helper has not been compiled or executed.
No factory workflow, VM, suite, artifact publication or public push ran. There
is no performance result. The consumer candidate's attempted Linux Rust check
stopped on this Mac's missing x86_64-linux-musl-gcc; it is not Linux compilation
evidence for that separate runtime change.

The helper source SHA-256 is
`28c4e5b5ed7fa0d6682031c5b33ea02c0460033227d495180b3cd9bc8a8c28fe`.
The unchanged builder-root/pins.json SHA-256 is
`9c3028025b6aa3ca8c31d02c3a26940ac3ddb97bfc5b5700ff6737ee0ece57ea`.

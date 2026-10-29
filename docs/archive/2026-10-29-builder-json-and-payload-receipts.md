# Ordinary producer JSON and payload bounds

Date: 2026-10-29. Production source remains `c4bcd4d`, executable
`fdefe37d46cade9d2e4b85c55b2f48ba63b0ad0e94cbd84fcd68090779e602b7`.
Both finite external drivers hash their actual executable and source.
They use the ordinary CLI and global cache, no limit override, runtime
replacement or synthetic output stream. Both used file-backed TMPDIR,
explicit signal cleanup and a new private evidence directory.

## JSON

The normal Dockerfile frontend accepts many short literal LABEL lines.
Twenty distinct 32 KiB values from scratch exceeded the cumulative 1 MiB
JSON bound. The CLI returned failed/exit 1 with `image-output: JSON limit`
after 12.229 s; public references stayed unchanged. Twelve values at the
same recipe path and CLI command built successfully in 15.436 s. Its
actual config is 788,048 bytes, with 393,432 history CreatedBy bytes;
index, manifest and config total 788,628 bytes. It has zero layers.
The literal values are fixture ASCII, not credentials.

The valid config and manifest were copied as already validated immutable
metadata. No layer was unpacked and no guest-writable disk was read.
The fixture's exact image name was then removed through the public CLI.
It leaves no large config in later discovery snapshots. The separate
caller-label output omission is recorded as a reproduced bug, not a
failure of the JSON guard or a new reserved-label policy.

## Aggregate output

Two normal RUN instructions each wrote 300 MiB from the guest kernel's
`/dev/urandom`. The second removed the first file before writing another;
the final random file would therefore be 300 MiB. The host rejected the
actual producer stream with `image-output: payload limit`, failed/exit 1,
after 36.545 s. Public references stayed unchanged. This guard runs before
conversion. The exact rejected stream length was not retained.

The same CLI command with 50 MiB in each RUN built in 25.954 s. Its actual
published index and unique blobs total 105,999,166 bytes. The two new
random layers are 52,444,977 and 52,445,002 bytes; each real compressed
size was measured from the accepted immutable objects. The driver copied
only JSON metadata and blob size observations, not the large layer data.
It then removed only its unique owned image through the public CLI.

Both controls published their reported unique/latest references together.
Both cleanup commands succeeded. These are manual guard/recovery receipts,
not new canonical tests, whole-suite passes or numeric policy approval.
They do not cover wrong hashes, malformed graphs, unsupported media,
descriptor URLs, attachments, frame-header size or object-count rejection.

Evidence is copied out under
`/Users/adam/Code/pinfold/target/phase6b-builder-integration-2026-10-28/`.

| Bundle or input | SHA-256 |
|---|---|
| `json-limit-success.tar.gz` | `ae6416cec24b71b36a8c47a92bf5175a6bab6615f15e58446cff7fb972186ec7` |
| JSON driver | `35f6bcf60bcd1849d89634a659330be5d41138459871255b2d7573a30dd3fb8a` |
| `output-limit-success.tar.gz` | `cd3ed3a05e7ebe013b6063f9ce13490c0c78a90e9b15b74f9316670683842a77` |
| Aggregate-output driver | `c5dc95498a6b70b0b37b5161342413b9bcac885b4529c6251caaca65e443374f` |

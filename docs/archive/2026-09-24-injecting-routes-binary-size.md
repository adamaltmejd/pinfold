# 2026-09-24: binary size after the TLS client (Y-42)

Injecting routes (guarantee 21, commit ce4cf5d) added the first TLS
client: `rustls` 0.23 with the `ring` provider and `rustls-native-certs`
for the host's roots. `cargo tree -i aws-lc-rs` is empty on both hosts.

Release `pinfold`, bytes, before and after ce4cf5d:

| Target | Before | After | Delta |
|---|---|---|---|
| aarch64-unknown-linux-gnu, as built (measured in the lane) | 2,387,904 | 4,136,600 | +1,748,696 |
| aarch64-unknown-linux-gnu, stripped (measured in the lane) | 1,643,400 | 2,954,136 | +1,310,736 |
| aarch64-apple-darwin, as built (`cargo build --release`) | 3,804,864 | 6,483,296 | +2,678,432 |
| x86_64-unknown-linux-musl, as built (labvm) | not measured | 4,389,704 | |

The darwin delta is larger than the plan's estimate of 1.0 to 1.5 MB
because the macOS binary keeps its symbols; the stripped Linux figure is
the one to compare with the estimate. The darwin "before" is main at
deef8df rather than the lane's base b3a4e7a; the two commits in between
(Y-43, a config-only commit) add no dependency.

# Project overview and profile conflicts, 2026-11-18

Built the CLI with `cargo build --locked` on Apple Silicon. Commands below
used that binary. The existing project's owner was running the previously
installed binary.

| Command and setup | Observed |
|---|---|
| `pinfold` in a fresh Git checkout with separate, absent XDG roots | Reported stopped, the proposed default profile, shared files and the managed image. Left all three roots absent. |
| `PINFOLD_PROFILE=documents pinfold` in that checkout | Previewed the documents image and the conditional build. |
| `PINFOLD_PROFILE=ready pinfold` in an unconfigured folder, with a saved profile specifying only an image | Named the saved profile and explained the local image prerequisite. |
| `VISUAL=true pinfold config edit`, then `pinfold` and `pinfold config --json` in the fixture checkout | Reported stopped. The image and mount matched the configuration. |
| `pinfold start`, `pinfold`, `pinfold stop`, `pinfold` in the fixture | Reported running with profile default and then stopped. Removed the fixture home disk after the run. |
| `PATH=/nonexistent PINFOLD_PROFILE=full pinfold` in the live project | Reported a running default owner, the full override, the saved default selection, image and config path. |
| `PINFOLD_PROFILE=full pinfold -- sh -c 'echo UNEXPECTED'` against that owner | Exited 125 before the command ran. Named both profiles, the variable to unset and the differing fields. |
| `env -u PINFOLD_PROFILE pinfold -- true` against the same owner | Exited 0. |
| `pinfold --` | Exited 125 and asked for a command. |

`cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings`
passed. The same Clippy check passed for `--target aarch64-unknown-linux-musl`
using Zig as the C cross compiler. Runtime observations above are from macOS.

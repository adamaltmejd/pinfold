# Interactive project setup on macOS

The installed build at 36a16f4 refused a first interactive launch with
no-config. In the operator's terminal, config edit then created a file but
the configured editor was unavailable. The selected user profile successfully
built an older recipe that omitted Pi. The guest exec error did not identify
the image as the place to look.

On the usability branch, cargo fmt --check, cargo build --locked and
cargo clippy --all-targets --locked -- -D warnings passed on arm64 macOS.
The following cases used the resulting binary, Apple Container and a real
pseudo-terminal in a temporary Git project with private XDG directories.
The custom profile had a unique name and a Containerfile based on the local
default image with a fixture label.

| Command and setup | Observed |
|---|---|
| Bare `pinfold --` | Exit 125 with shell and Pi examples, before setup. |
| `pinfold -- nonsense` with redirected streams and no config | Exit 125, no-config. |
| `pinfold -- bash`, decline the mount | Exit 125, no config file. |
| `pinfold -- pi`, interrupt at the profile question | Exit 130, no config file. |
| `PINFOLD_PROFILE=default pinfold -- nonsense` with no config | Identified the bundled profile and environment selection. Enter accepted the mount. Guest command lookup failed with exit 125 and an image diagnostic. |
| Select the custom profile, confirm setup, decline the image build | Exit 125. The profile was saved. In a separate trial the next interactive launch built it successfully. |
| Fresh config and missing custom image. Run `pinfold -- sh -c 'printf "%s" "$1" > here' sh 'a b;$x'` | Accepted mount and build with Enter. One invocation built the image and exited 0. The host file contained the literal argument. |
| `pinfold -- bash -c 'exit 23'` | Reused setup and passed through 23. |
| Set a missing explicit image, run `pinfold -- pi` | Exit 125 with image-missing and no build question. |
| `pinfold typo`, `pinfold box exec abc`, and `VISUAL=pinfold-fixture-missing-editor pinfold config edit` | Help pointer, leaf-specific usage, and editor recovery guidance respectively. The last returned 127. |

The fixture stopped its project, removed its home disk and its image tags,
and removed its temporary directories. The Linux runtime and an authenticated
model request were outside this check.

# Macmini CI disk cleanup

Macmini is shared by Adam and his wife. Its Apple container runtime is
used only for pinfold CI. Cleanup is limited to that runtime and pinfold's
end-to-end artifact cache. It does not clean shared Homebrew caches,
personal files, Rust toolchains, or logins.

Homebrew build tools are installed under `/opt/homebrew` and available
to other users whose PATH includes them. Rust toolchains and Cargo data
are under the runner account's `~/.rustup` and `~/.cargo`. Container data
is under that account's Library. These are persistent host installations,
not job-local tools.

The nightly Mac job runs `scripts/clean-mac-runner.sh` before the suite
and after success or failure. It removes leftover running or stopped
containers except `buildkit`, removes unused images, and prunes unused
BuildKit cache with a 4096 MB retention target. Removing the default
profile image makes the next suite build the candidate's embedded pins.

Existing artifact maintenance prunes obsolete harness versions using
isolated configuration and state. Cleanup also removes obsolete embedded
init artifacts and resets the dedicated test cache if it exceeds 2 GiB.
It honors TMPDIR, matching the suite. Builds require 20 GiB free.

The limits are cleanup targets, not filesystem quotas. Active references
and sparse VM storage can differ from the builder's reported cache size.
Hard interruption can prevent post-job cleanup; the next job also cleans
before building. Shared host storage is outside this policy.

## Verification

The first real-host cleanup reclaimed about 20 GB of unused images.
Available APFS data-volume space increased from about 28 GiB to 47 GiB.
After the full suite and its cleanup, about 46 GiB remained free.
The retained container directory was 4.4 GB and test cache was 816 MB.

At `dae05ca`, the Mac workflow passed all 27 end-to-end tests, including
live login, in 254.93 seconds. Both pre-job and post-job cleanup passed.
Post-job cleanup reclaimed 14.18 GB of images.
[Mac run](https://github.com/adamaltmejd/pinfold/actions/runs/36677298570).
Both Linux architecture jobs also passed.
[Linux run](https://github.com/adamaltmejd/pinfold/actions/runs/36677298506).

Final refinements honor TMPDIR and remove running containers left by
interrupted jobs. The final script ran successfully on Macmini. A real
detached Debian container was removed while the builder survived. Cleanup
also passed with the builder stopped, then the builder was restarted.
These refinements received focused real-runtime checks; the full suite
was not repeated after them.

Shellcheck, shfmt, actionlint, `git diff --check`, `cargo fmt --check`, and
`cargo clippy --all-targets --locked -- -D warnings` passed. Automatic
releases remain disabled; this run changed no pins and published no release.

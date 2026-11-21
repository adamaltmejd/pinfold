//! The end-to-end suite's one test binary: one test per row of
//! ARCHITECTURE.md's Guarantees table. It runs on macOS with Apple
//! `container`, through scripts/run-mac-e2e.sh, or on Linux x64 with
//! Firecracker.

// On macOS `clean` prunes the host-wide BuildKit cache and keeps two builds
// of every source in the host-wide image store, the user's included; on
// Linux both live in the test's own cache root.
//
// libtest starts tests in name order. The modules whose tests build their
// own small images (build, chain, clean) sort before every test that waits
// for the default image, so they run beside its build, not after it.
#[cfg(target_os = "linux")]
mod build;
mod chain;
#[cfg(target_os = "linux")]
mod clean;
mod credentials;
mod crossing;
mod disk;
#[cfg(target_os = "linux")]
mod jail;
mod network;
mod owner;
mod privilege;
mod records;
mod sessions;
mod share;

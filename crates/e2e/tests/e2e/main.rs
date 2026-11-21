//! The end-to-end suite's one test binary: one test per row of
//! ARCHITECTURE.md's Guarantees table. It runs on macOS with Apple
//! `container`, through scripts/run-mac-e2e.sh, or on Linux x64 with
//! Firecracker.

// On macOS `clean` prunes the host-wide BuildKit cache and keeps two builds
// of every source in the host-wide image store, the user's included; on
// Linux both live in the suite's cache root.
#[cfg(target_os = "linux")]
mod clean;
mod credentials;
mod crossing;
mod disk;
mod network;
mod owner;
mod privilege;
mod records;
mod sessions;
mod share;

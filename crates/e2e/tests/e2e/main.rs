//! The end-to-end suite's one test binary: one test per row of
//! ARCHITECTURE.md's Guarantees table. It runs on macOS with Apple
//! `container`, through scripts/run-mac-e2e.sh, or on Linux x64 with
//! Firecracker.

mod disk;
mod network;
mod owner;
mod privilege;

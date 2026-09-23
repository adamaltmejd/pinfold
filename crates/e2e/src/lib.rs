//! End-to-end tests that drive the `pinfold` binary from outside.
//!
//! The tests live in `tests/` and run on a macOS host with the Apple
//! `container` CLI. `cargo test -p e2e` builds the binary and runs them, so
//! that one command is the whole host gate.

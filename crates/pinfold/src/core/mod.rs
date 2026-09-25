//! pinfold's core: box specs, profiles, runtimes and the box lifecycle.

pub mod artifacts;
pub mod r#box;
pub mod clean;
pub mod image;
pub mod network;
pub mod plan;
pub mod profile;
pub mod proxy;
pub mod runtime;
pub mod tls;

/// `bytes` as lowercase hex. Ids and cache paths on disk are made of it.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

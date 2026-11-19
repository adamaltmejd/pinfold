//! The pi layer: per-project state and disks, the box launch, and the Git
//! remote of a disk project.

pub mod context;
pub mod launch;
pub mod remote;
pub mod state;

pub(crate) mod session;
pub(crate) mod session_snapshot;
pub(crate) mod session_wire;

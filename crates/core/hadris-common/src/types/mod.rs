//! Contains utility types commonly used for filesystems.

pub mod endian;
pub mod extent;
/// Endian-aware integer types and alignment helpers.
pub mod number;

/// Layout types for metadata-only writing (requires `alloc` feature).
#[cfg(feature = "alloc")]
pub mod layout;

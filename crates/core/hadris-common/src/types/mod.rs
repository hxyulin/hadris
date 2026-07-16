//! Contains utility types commonly used for filesystems.

pub mod endian;
pub mod extent;
pub mod file;
/// Fixed-capacity representations for allocation-free parsing.
pub mod no_alloc;
/// Endian-aware integer types and alignment helpers.
pub mod number;

/// Layout types for metadata-only writing (requires `alloc` feature).
#[cfg(feature = "alloc")]
pub mod layout;

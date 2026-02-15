//! Contains utility types commonly used for filesystems.

pub mod endian;
pub mod extent;
pub mod file;
pub mod no_alloc;
pub mod number;

/// Layout types for metadata-only writing (requires `alloc` feature).
#[cfg(feature = "alloc")]
pub mod layout;

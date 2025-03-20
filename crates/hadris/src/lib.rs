//! Hadris is a unified package containing different implementations for file systems.
//!
//! Currently, the only supported file system is the ISO-9660 filesystem.

#[cfg(feature = "iso9660")]
pub use hadris_iso as iso;

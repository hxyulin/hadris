//! Hadris is a unified package containing different implementations for file systems.

#[cfg(feature = "iso9660")]
pub use hadris_iso as iso;

#[cfg(feature = "fat")]
pub use hadris_fat as fat;

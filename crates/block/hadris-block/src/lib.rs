//! Block-storage entry point for Hadris.
//!
//! This crate groups format-neutral block-device interfaces, partition tables,
//! and block filesystems without hiding their concrete APIs.

#![no_std]

/// Format-neutral block geometry and device capabilities.
#[cfg(feature = "storage")]
pub use hadris_storage as storage;

/// FAT12/16/32 and exFAT filesystem support.
#[cfg(feature = "fat")]
pub use hadris_fat as fat;

/// MBR, GPT, and hybrid partition-table support.
#[cfg(feature = "part")]
pub use hadris_part as part;

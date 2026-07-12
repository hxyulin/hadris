//! # Hadris
//!
//! A unified package for working with block devices, optical media, and archives.
//! The individual format crates are grouped by their storage access model:
//!
//! - [`block`] — block filesystems and partition tables
//! - [`optical`] — optical filesystems and disc image composition
//! - [`archive`] — sequential archive formats
//!
//! # Feature flags
//!
//! Leaf features (`fat`, `part`, `iso`, `udf`, `cd`, and `cpio`) enable one
//! library at a time. The `block`, `optical`, and `archive` features enable all
//! libraries in their respective category. Platform (`std`, `alloc`), I/O mode
//! (`sync`, `async`), and capability (`read`, `write`) features are forwarded
//! independently to enabled leaves. The default set is the hosted synchronous
//! read/write configuration with `fat`, `iso`, and `cpio`.
//!
//! Hybrid CD image creation is currently sync-only. Enabling `cd`—directly or
//! through `optical`—therefore enables the CD writer's sync API, even when the
//! umbrella `async` feature is also selected. ISO and UDF still expose their
//! async modules in that configuration.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use hadris::optical::iso::sync::IsoImage;
//!
//! let file = std::fs::File::open("image.iso").unwrap();
//! let iso = IsoImage::open(file).unwrap();
//! let pvd = iso.read_pvd();
//! println!("Volume: {}", pvd.volume_identifier);
//! ```

/// Block-oriented filesystems and disk-layout formats.
#[cfg(any(feature = "fat", feature = "part"))]
pub mod block {
    /// FAT12/16/32 and exFAT filesystem support.
    #[cfg(feature = "fat")]
    pub use hadris_fat as fat;

    /// MBR, GPT, and hybrid partition-table support.
    #[cfg(feature = "part")]
    pub use hadris_part as part;
}

/// Optical filesystems and disc-image composition.
#[cfg(any(feature = "iso", feature = "udf", feature = "cd"))]
pub mod optical {
    /// ISO 9660 filesystem support.
    #[cfg(feature = "iso")]
    pub use hadris_iso as iso;

    /// Universal Disk Format filesystem support.
    #[cfg(feature = "udf")]
    pub use hadris_udf as udf;

    /// Hybrid ISO+UDF optical-disc image creation.
    #[cfg(feature = "cd")]
    pub use hadris_cd as cd;
}

/// Sequential archive formats.
#[cfg(feature = "cpio")]
pub mod archive {
    /// CPIO newc/CRC archive support.
    pub use hadris_cpio as cpio;
}

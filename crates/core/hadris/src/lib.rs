//! # Hadris
//!
//! A unified package for working with block devices, optical media, and archives.
//! The individual format crates are grouped by their storage access model:
//!
//! - [`block`] — block filesystems and partition tables
//! - [`optical`] — optical filesystems and disc image composition
//! - [`archive`] — sequential archive formats
//! - [`path`] — lexical virtual-path parsing and normalization
//! - [`fixed`] — fixed-capacity byte and text types
//!
//! # Feature flags
//!
//! Leaf features (`storage`, `fat`, `part`, `iso`, `udf`, `cd`, and `cpio`) enable one
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
//! let pvd = iso.read_pvd().unwrap();
//! println!("Volume: {}", pvd.volume_identifier);
//! ```

/// Block-oriented storage, filesystems, and disk-layout formats.
#[cfg(any(feature = "storage", feature = "fat", feature = "part"))]
pub use hadris_block as block;

/// Optical filesystems and disc-image composition.
#[cfg(any(feature = "iso", feature = "udf", feature = "cd"))]
pub use hadris_optical as optical;

/// Sequential archive formats.
#[cfg(feature = "cpio")]
pub use hadris_archive as archive;

/// Lexical virtual-path utilities.
#[cfg(feature = "path")]
pub use hadris_path as path;

/// Fixed-capacity byte and text types.
#[cfg(feature = "fixed")]
pub use hadris_fixed as fixed;

//! # Hadris
//!
//! **The Rust storage stack.**
//!
//! Hadris is a pure Rust toolkit for block devices, partition tables,
//! filesystems, archives, and disk images. Its feature model supports desktop
//! applications and `no_std` bootloaders, kernels, firmware, and embedded
//! systems.
//!
//! The umbrella crate groups the individual libraries by storage access model:
//!
//! - [`block`] — block filesystems and partition tables
//! - [`optical`] — optical filesystems and disc image composition
//! - [`cpio`] — CPIO newc and CRC archives
//! - [`fs`] — shared filesystem vocabulary and lexical virtual paths
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
//! `async-send` implies `async` and adds the `async_send` modules of
//! `hadris-io`, `hadris-fs`, `hadris-storage`, `hadris-fat` and
//! `hadris-block`, whose futures are `Send` for multi-threaded executors. The
//! optical and archive crates have no such mode yet.
//!
//! Hybrid CD image creation is currently sync-only. Enabling `cd`—directly or
//! through `optical`—therefore enables the CD writer's sync API, even when the
//! umbrella `async` feature is also selected. ISO and UDF still expose their
//! async modules in that configuration.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use hadris::io::StdIo;
//! use hadris::optical::iso::sync::IsoImage;
//!
//! let file = StdIo::new(std::fs::File::open("image.iso").unwrap());
//! let iso = IsoImage::open(file).unwrap();
//! let pvd = iso.read_pvd().unwrap();
//! println!("Volume: {}", pvd.volume_identifier);
//! ```

#![deny(missing_docs)]

/// I/O traits, errors and adapters shared by every format.
pub use hadris_io as io;

/// Block-oriented storage, filesystems, and disk-layout formats.
#[cfg(any(feature = "storage", feature = "fat", feature = "part"))]
pub use hadris_block as block;

/// Optical filesystems and disc-image composition.
#[cfg(any(feature = "iso", feature = "udf", feature = "cd"))]
pub use hadris_optical as optical;

/// CPIO newc and CRC archives.
#[cfg(feature = "cpio")]
pub use hadris_cpio as cpio;

/// Shared filesystem vocabulary and lexical virtual paths.
#[cfg(feature = "fs")]
pub use hadris_fs as fs;

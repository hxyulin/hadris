//! # Hadris ISO
//!
//! ISO 9660 images: an allocation-free reader for every tree an image can
//! carry, and a writer that builds images from a shared input tree.
//!
//! ## Reading
//!
//! `IsoImage` opens an image on a `hadris_storage` block device, in each
//! mode (`sync::IsoImage`, `r#async::IsoImage`). An
//! image has up to four trees: the primary tree, Rock Ridge names and
//! metadata over it, a Joliet tree and an ISO 9660:1999 enhanced tree.
//! `view` picks one as an `IsoView`, which implements the read-only
//! `hadris_fs` `FileSystem` trait, so `Volume` and its handles work on it.
//! Reading needs no allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::io::Read;
//!
//! use hadris_fs::sync::Volume;
//! use hadris_fs::{Content, Node, OpenOptions, Tree};
//! use hadris_iso::sync::{IsoImage, write};
//! use hadris_iso::{IsoOptions, Namespace, plan};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut tree = Tree::new();
//! tree.insert("boot/grub/grub.cfg", Node::file(Content::bytes("set timeout=3")))?;
//! let options = IsoOptions::default();
//! let size = plan(&tree, &options)?.size();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! let report = write(&mut dev, &tree, &options)?;
//! assert_eq!(report.size(), size);
//!
//! let iso = IsoImage::open(dev)?;
//! let vol = Volume::new(iso.into_view(Namespace::Preferred)?);
//! let mut text = String::new();
//! vol.open("/boot/grub/grub.cfg", OpenOptions::new().read())?
//!     .read_to_string(&mut text)?;
//! assert_eq!(text, "set timeout=3");
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! `IsoImage::boot_catalog` reads the El Torito catalog, `IsoView::rock_ridge`
//! the Rock Ridge entries of a node and `IsoView::raw_record` its directory
//! record. The on-disk layouts are in [`raw`].
//!
//! ## Writing
//!
//! `write` (in each mode, with `alloc`) lays out a `hadris_fs::Tree` as an
//! image on a block device, as [`IsoOptions`] says, and returns a
//! `hadris_fs::Report` of its size, where each file went, and what it could
//! not store. [`plan`] returns the same report without I/O, so the device
//! can be sized first. The output is reproducible: the writer reads no
//! clock, and dates the volume with [`IsoOptions::with_time`], 1980-01-01
//! by default.
//!
//! ## Sessions
//!
//! `Session` (in each mode, with `alloc`) reads an existing image into a
//! tree whose files point back at their extents, and writes it back after
//! changes: as a new session after the old one ([`SessionMode::Append`]),
//! or rebuilt in place ([`SessionMode::Rewrite`]).
//!
//! Reading fails with [`hadris_fs::Error`]; [`Detail::of`] names the
//! structure or option at fault, and a device whose first volume descriptor
//! is not ISO 9660 fails with
//! [`ErrorKind::NotRecognized`](hadris_fs::ErrorKind::NotRecognized).
//! Writing fails with [`hadris_fs::PathError`], which carries the path of
//! the file whose content failed; [`Detail::from_code`] reads its detail.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
//! | `alloc` | via `std` | The writer, sessions, `BootCatalog` and the `Tree` input |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API with `Send` futures in `r#async` |
//!
//! No feature changes what an item does.

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    not(all(feature = "alloc", any(feature = "sync", feature = "async"))),
    allow(dead_code)
)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

mod boot;
mod error;
mod info;
mod name;
mod namespace;
#[cfg(feature = "alloc")]
mod options;
#[cfg(feature = "alloc")]
mod plan;
mod rock_ridge;

pub mod raw;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[cfg(feature = "alloc")]
    use hadris_fs::sync as fs;
    #[cfg(feature = "alloc")]
    use hadris_part::sync as part;
    use hadris_storage::sync as storage;

    use hadris_fs::sync::FileSystem;

    #[path = "image.rs"]
    mod image;
    pub use image::{IsoImage, IsoView};
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::write;
    #[cfg(feature = "alloc")]
    #[path = "session.rs"]
    mod session;
    #[cfg(feature = "alloc")]
    pub use session::Session;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod r#async;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use boot::BootCatalog;
pub use boot::{BootCatalogEntry, Emulation, Platform};
pub use error::Detail;
pub use namespace::{JolietLevel, Namespace, Namespaces};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use options::{
    BootEntry, BootInfo, Charset, ElTorito, HybridBoot, IsoLevel, IsoOptions, NameCase,
    PartitionScheme, Preserve, Relocation, RockRidge, SessionMode, VolumeIdentifiers,
};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use plan::plan;
pub use rock_ridge::RockRidgeInfo;

#[cfg(test)]
extern crate self as hadris_iso;

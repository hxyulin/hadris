//! # Hadris ISO
//!
//! ISO 9660 images: an allocation-free reader for every tree an image can
//! carry, and a writer that builds images from a shared input tree.
//!
//! ## Reading
//!
//! `IsoImage` opens an image on a `hadris_storage` block device, in each
//! mode (`sync::IsoImage`, `r#async::IsoImage`, `async_send::IsoImage`). An
//! image has up to four trees: the primary tree, Rock Ridge names and
//! metadata over it, a Joliet tree and an ISO 9660:1999 enhanced tree.
//! `view` picks one as an `IsoView`, which implements the `hadris_fs`
//! `FsDriver` trait, so the path helpers, `Volume` and handles of
//! `hadris-fs` work on it. Reading needs no allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fs::sync::DriverExt;
//! use hadris_fs::tree::{Content, Tree};
//! use hadris_iso::sync::{IsoImage, plan, write};
//! use hadris_iso::{IsoOptions, Namespace};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut tree = Tree::new();
//! tree.add_file("boot/grub/grub.cfg", Content::bytes("set timeout=3"))?;
//! let options = IsoOptions::default();
//! let size = plan(&tree, &options)?.size_bytes();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! let report = write(&mut dev, &tree, &options)?;
//! assert_eq!(report.size_bytes(), size);
//!
//! let mut iso = IsoImage::open(dev)?;
//! let mut view = iso.view(Namespace::Preferred)?;
//! assert_eq!(view.read_to_vec("/boot/grub/grub.cfg")?, b"set timeout=3");
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
//! `write` (in each mode, with `alloc`) lays out a `hadris_fs::tree::Tree`
//! as an image on a block device, as [`IsoOptions`] says, and returns a
//! [`Report`] of its size, where each file went, and what it could not
//! store. `plan` returns the same report without writing, so the device can
//! be sized first. The output is reproducible: the clock is injected, and
//! the default [`NoClock`](hadris_fs::NoClock) writes 1980-01-01.
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
//! | `async` | No | The asynchronous API in `r#async` |
//! | `async-send` | No | The asynchronous API with `Send` futures in `async_send` |
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
#[cfg(feature = "alloc")]
mod report;
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

    macro_rules! impl_iso_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    #[path = "image.rs"]
    mod image;
    pub use image::{IsoImage, IsoView};
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{plan, write};
    #[cfg(feature = "alloc")]
    #[path = "session.rs"]
    mod session;
    #[cfg(feature = "alloc")]
    pub use session::Session;
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    //! The asynchronous API, generated from the same source as `sync`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    #[cfg(feature = "alloc")]
    use hadris_fs::r#async as fs;
    #[cfg(feature = "alloc")]
    use hadris_part::r#async as part;
    use hadris_storage::r#async as storage;

    macro_rules! impl_iso_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    #[path = "image.rs"]
    mod image;
    pub use image::{IsoImage, IsoView};
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{plan, write};
    #[cfg(feature = "alloc")]
    #[path = "session.rs"]
    mod session;
    #[cfg(feature = "alloc")]
    pub use session::Session;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated a third time from the same source.
#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
pub mod async_send;

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
pub use report::Report;
pub use rock_ridge::RockRidgeInfo;

#[cfg(test)]
extern crate self as hadris_iso;

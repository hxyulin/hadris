//! # Hadris UDF
//!
//! Universal Disk Format (ECMA-167 and the OSTA UDF specification) volumes:
//! an allocation-free reader and a writer that builds volumes from a shared
//! input tree.
//!
//! ## Reading
//!
//! `UdfFs` opens a volume on a `hadris_storage` block device, in each mode
//! (`sync::UdfFs`, `r#async::UdfFs`). It implements
//! the `hadris_fs` `FileSystem` trait read-only, so `Volume` and its
//! handles work on it. Node ids are ICB
//! locations and need no node table. Reading needs no allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::io::Read;
//!
//! use hadris_fs::sync::Volume;
//! use hadris_fs::{Content, MountOptions, Node, OpenOptions, Tree};
//! use hadris_storage::{BlockSize, MemDevice};
//! use hadris_udf::{UdfId, UdfOptions, plan};
//! use hadris_udf::sync::{UdfFs, write};
//!
//! let mut tree = Tree::new();
//! tree.insert("docs/readme.txt", Node::file(Content::bytes("hello")))?;
//! let options = UdfOptions::new().with_id(UdfId::Volume, "DOCS");
//! let size = plan(&tree, &options)?.size();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! write(&mut dev, &tree, &options)?;
//!
//! let udf = UdfFs::mount(dev, MountOptions::new())?;
//! assert_eq!(udf.info().id(UdfId::Volume), "DOCS");
//! let vol = Volume::new(udf);
//! let mut text = String::new();
//! vol.open("/docs/readme.txt", OpenOptions::new().read())?.read_to_string(&mut text)?;
//! assert_eq!(text, "hello");
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! The reader handles UDF 1.02 to 2.01 volumes with type 1 partitions:
//! logical blocks of 512 to 4096 bytes, the prevailing descriptors of the
//! main or reserve sequence, file entries and extended file entries,
//! short, long, extended and embedded allocation descriptors with
//! continuation extents, symbolic links and hard links. Virtual, sparable
//! and metadata partitions (UDF 1.50 packet writing and 2.50) are refused
//! as [`ErrorKind::Unsupported`](hadris_fs::ErrorKind::Unsupported).
//! The on-disk layouts are in [`raw`].
//!
//! ## Writing
//!
//! `write` (in each mode, with `alloc`) lays out a `hadris_fs::Tree` as a
//! UDF volume on a block device, as [`UdfOptions`] says, and returns a
//! `hadris_fs::Report` of its size, where each file went, and what it could
//! not store. [`plan`] returns the same report without I/O, so the device
//! can be sized first. The output is reproducible: the writer reads no
//! clock, and dates the volume with [`UdfOptions::with_time`], 1980-01-01
//! by default.
//!
//! `write_bridge` writes an ISO 9660 and UDF bridge image, as DVD-Video
//! uses: both file systems point at the same file data. It takes the
//! `hadris_iso::IsoOptions` of the ISO 9660 half, and [`plan_bridge`]
//! returns its report without I/O.
//!
//! Reading fails with [`hadris_fs::Error`]; [`Detail::of`] names the
//! structure or option at fault, and a device without a UDF recognition
//! sequence fails with
//! [`ErrorKind::NotRecognized`](hadris_fs::ErrorKind::NotRecognized).
//! Writing fails with [`hadris_fs::PathError`], which carries the path of
//! the file whose content failed; [`Detail::from_code`] reads its detail.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
//! | `alloc` | via `std` | The writers, `plan` and `plan_bridge` |
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

#[cfg(feature = "alloc")]
mod bridge;
mod error;
mod name;
#[cfg(feature = "alloc")]
mod options;
#[cfg(feature = "alloc")]
mod plan;
mod revision;
mod time;
mod volume;

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
    use hadris_iso::sync as iso;
    use hadris_storage::sync as storage;

    use hadris_fs::sync::FileSystem;

    #[path = "read.rs"]
    mod read;
    pub use read::UdfFs;
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{write, write_bridge};
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod r#async;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use bridge::plan_bridge;
pub use error::Detail;
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use options::UdfOptions;
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use plan::plan;
pub use revision::UdfRevision;
pub use volume::{EntityId, PartitionInfo, PartitionKind, UdfId, VolumeInfo};

#[cfg(test)]
extern crate self as hadris_udf;

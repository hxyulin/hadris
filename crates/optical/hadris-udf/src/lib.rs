//! # Hadris UDF
//!
//! Universal Disk Format (ECMA-167 and the OSTA UDF specification) volumes:
//! an allocation-free reader and a writer that builds volumes from a shared
//! input tree.
//!
//! ## Reading
//!
//! `UdfFs` opens a volume on a `hadris_storage` block device, in each mode
//! (`sync::UdfFs`, `r#async::UdfFs`, `async_send::UdfFs`). It implements
//! the `hadris_fs` `FsDriver` trait read-only, so the path helpers,
//! `Volume` and handles of `hadris-fs` work on it. Node ids are ICB
//! locations and need no node table. Reading needs no allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fs::sync::DriverExt;
//! use hadris_fs::tree::{Content, Tree};
//! use hadris_storage::{BlockSize, MemDevice};
//! use hadris_udf::UdfOptions;
//! use hadris_udf::sync::{UdfFs, plan, write};
//!
//! let mut tree = Tree::new();
//! tree.add_file("docs/readme.txt", Content::bytes("hello"))?;
//! let options = UdfOptions::default().with_volume_id("DOCS");
//! let size = plan(&tree, &options)?.size_bytes();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! write(&mut dev, &tree, &options)?;
//!
//! let mut udf = UdfFs::open(dev)?;
//! assert_eq!(udf.volume_id(), "DOCS");
//! assert_eq!(udf.read_to_vec("/docs/readme.txt")?, b"hello");
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
//! `write` (in each mode, with `alloc`) lays out a `hadris_fs::tree::Tree`
//! as a UDF volume on a block device, as [`UdfOptions`] says, and returns a
//! [`Report`] of its size, where each file went, and what it could not
//! store. `plan` returns the same report without writing, so the device can
//! be sized first. The output is reproducible: the clock is injected, and
//! the default [`NoClock`](hadris_fs::NoClock) writes 1980-01-01. With
//! [`Bridge`], the volume shares an image with ISO 9660 and points at file
//! data already on the device, as `hadris-cd` does.
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
//! | `alloc` | via `std` | The writer and the `Tree` input |
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

mod error;
mod name;
#[cfg(feature = "alloc")]
mod options;
#[cfg(feature = "alloc")]
mod plan;
#[cfg(feature = "alloc")]
mod report;
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
    use hadris_storage::sync as storage;

    macro_rules! impl_udf_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    #[path = "read.rs"]
    mod read;
    pub use read::UdfFs;
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{plan, write};
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
    use hadris_storage::r#async as storage;

    macro_rules! impl_udf_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    #[path = "read.rs"]
    mod read;
    pub use read::UdfFs;
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{plan, write};
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated a third time from the same source.
#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
pub mod async_send;

pub use error::Detail;
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use options::{Bridge, UdfOptions};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use report::Report;
pub use revision::UdfRevision;
pub use volume::Partition;

#[cfg(test)]
extern crate self as hadris_udf;

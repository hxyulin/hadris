//! # Hadris NTFS
//!
//! A read-only NTFS reader that needs no allocator.
//!
//! `NtfsFs` opens a volume on a `hadris_storage` block device, in each mode
//! (`sync::NtfsFs`, `r#async::NtfsFs`). It implements the `hadris_fs`
//! `FileSystem` trait read-only, so `Volume` and its handles work on it.
//! Node ids are file references and need no node table.
//!
//! ```rust,no_run
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::io::Read;
//!
//! use hadris_fs::OpenOptions;
//! use hadris_fs::sync::Volume;
//! use hadris_ntfs::sync::NtfsFs;
//!
//! let image = hadris_storage::host::FileDevice::open("disk.img")?;
//! let vol = Volume::new(NtfsFs::open(image)?);
//! for entry in vol.read_dir("/")? {
//!     let entry = entry?;
//!     println!("{:?}", entry.name());
//! }
//! let mut readme = Vec::new();
//! vol.open("/docs/readme.txt", OpenOptions::new().read())?.read_to_end(&mut readme)?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! Beyond the trait, `NtfsFs` reads named data streams
//! (`streams`, `read_stream_at`).
//!
//! ## Supported scope
//!
//! The reader checks the boot sector and update sequence arrays, reads
//! resident, non-resident, sparse and partly initialized streams, follows
//! `$ATTRIBUTE_LIST` entries into extension records (also for `$MFT`, up to
//! 32 extents), walks directory indexes through their allocation bitmaps,
//! compares Win32 names through the volume's `$UpCase` table and POSIX
//! names exactly, and checks the sequence numbers of file references. MFT
//! and index records of up to 4096 bytes and device blocks of up to 4096
//! bytes are supported. Listings leave out DOS aliases and the metadata
//! files, which a lookup by name still finds.
//!
//! It does not yet recover from `$MFTMirr`, decode compressed or encrypted
//! streams (they fail with
//! [`ErrorKind::Unsupported`](hadris_fs::ErrorKind::Unsupported)), interpret
//! reparse points, read security descriptors, or descend the index B-tree
//! by key. NTFS is a preview in Hadris 3.0: the `FileSystem` implementation
//! follows the frozen trait, the native methods may still change. The
//! on-disk layouts are in [`raw`].
//!
//! Errors are [`hadris_fs::Error`]; [`Detail::of`] names the structure at
//! fault. A device without an NTFS boot sector fails with
//! [`ErrorKind::NotRecognized`](hadris_fs::ErrorKind::NotRecognized).
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; forwards `std` to `hadris-fs` and `hadris-storage` |
//! | `alloc` | via `std` | Forwards `alloc` to `hadris-fs` and `hadris-storage` |
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
#![cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

mod error;
mod record;
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

    use hadris_fs::sync::FileSystem;
    use hadris_storage::sync as storage;

    #[path = "fs.rs"]
    mod fs;
    pub use fs::NtfsFs;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod r#async;

pub use error::Detail;

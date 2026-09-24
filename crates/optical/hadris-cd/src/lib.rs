//! # Hadris CD
//!
//! Hybrid ISO 9660 and UDF optical disc images in the UDF Bridge format,
//! written from one `hadris_fs::tree::Tree`. Legacy systems read the
//! ISO 9660 tree, modern ones the UDF tree, and both point at the same
//! file data.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_cd::CdOptions;
//! use hadris_cd::sync::{plan, write};
//! use hadris_fs::tree::{Content, Tree};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut tree = Tree::new();
//! tree.add_file("readme.txt", Content::bytes("Hello, World!"))?;
//! let options = CdOptions::default();
//! let size = plan(&tree, &options)?.size_bytes();
//! let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(2048).unwrap());
//! let report = write(&mut dev, &tree, &options)?;
//! assert!(report.extent_of("readme.txt").is_some());
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! ## Layout
//!
//! ```text
//! Sector 0-15:    System area (hybrid boot code and partition tables)
//! Sector 16-...:  ISO 9660 volume descriptors, then the UDF recognition
//!                 sequence (BEA01, NSR02 or NSR03, TEA01)
//! Sector 256:     UDF anchor volume descriptor pointer
//! Sector 257-289: UDF volume descriptor sequences and integrity descriptor
//! Sector 290-...: UDF file set, file entries and directories
//! Then:           ISO 9660 directories, path tables and the file data,
//!                 which both trees point at
//! End:            UDF anchor at N-256 and 256 blocks after it
//! ```
//!
//! `write` plans the UDF metadata, writes the ISO 9660 image after it with
//! `hadris_iso`, then writes the UDF structures with `hadris_udf` in bridge
//! mode, pointing at the extents the ISO [`Report`](hadris_iso::Report)
//! gives. Nothing is read back from the device.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | `std::io::Error` conversions and host files as tree content |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API in `r#async` |
//! | `async-send` | No | The asynchronous API with `Send` futures in `async_send` |
//!
//! The crate needs an allocator. No feature changes what an item does.

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]

extern crate alloc;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

mod error;
mod options;
mod report;
mod tree;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_iso::sync as iso;
    use hadris_storage::sync as storage;
    use hadris_udf::sync as udf;

    #[path = "write.rs"]
    mod write;
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

    use hadris_iso::r#async as iso;
    use hadris_storage::r#async as storage;
    use hadris_udf::r#async as udf;

    #[path = "write.rs"]
    mod write;
    pub use write::{plan, write};
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated a third time from the same source.
#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
pub mod async_send;

pub use error::{Detail, Error};
pub use hadris_iso::IsoOptions;
pub use hadris_udf::UdfOptions;
pub use options::CdOptions;
pub use report::Report;

/// The ISO 9660 crate, for the option types of [`IsoOptions`].
pub use hadris_iso as iso;
/// The UDF crate, for the option types of [`UdfOptions`].
pub use hadris_udf as udf;

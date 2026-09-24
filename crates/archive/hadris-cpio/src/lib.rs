//! # Hadris CPIO
//!
//! cpio archives, as Linux initramfs, RPM payloads and `cpio(1)` use them:
//! an allocation-free streaming reader and a streaming writer that also
//! writes a shared input tree.
//!
//! ## Reading
//!
//! `CpioReader` (in each mode: `sync::CpioReader`, `r#async::CpioReader`,
//! `async_send::CpioReader`) reads `newc` (`070701`), `newc` with
//! checksums (`070702`), `odc` (`070707`) and old binary archives from any
//! `hadris_io` `Read` stream, such as a pipe. `next_entry` returns an
//! `Entry` that borrows the reader and implements `Read` over its data;
//! data left unread is skipped by the next call. Reading needs no
//! allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_cpio::sync::{CpioReader, write};
//! use hadris_cpio::CpioOptions;
//! use hadris_fs::tree::{Content, Tree};
//! use hadris_io::{Cursor, StdIo};
//! use hadris_io::sync::Read;
//!
//! let mut tree = Tree::new();
//! tree.add_file("etc/hostname", Content::bytes("hadris\n"))?;
//! let mut out = StdIo::new(Vec::new());
//! let report = write(&mut out, &tree, &CpioOptions::default())?;
//! let archive = out.into_inner();
//! assert_eq!(report.size_bytes(), archive.len() as u64);
//!
//! let mut reader = CpioReader::new(Cursor::new(&archive));
//! let mut names = Vec::new();
//! while let Some(mut entry) = reader.next_entry()? {
//!     names.push(entry.name_str()?.to_string());
//!     if entry.name() == b"etc/hostname" {
//!         let mut data = [0u8; 7];
//!         entry.read_exact(&mut data)?;
//!         assert_eq!(&data, b"hadris\n");
//!     }
//! }
//! assert_eq!(names, ["etc", "etc/hostname"]);
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! An archive that ends at an entry boundary without a `TRAILER!!!` entry
//! is valid, as the Linux initramfs format allows;
//! [`ReaderOptions::with_strict_trailer`] requires one.
//! `continue_after_trailer` reads archives concatenated after a trailer.
//!
//! ## Writing
//!
//! `CpioWriter` (with `alloc`) streams entries to a `Write` stream in
//! `newc`, `newc` with checksums or `odc` ([`Format`]): `append` writes one
//! file, directory, symlink, device node, FIFO or socket with its
//! `hadris_fs::SetMetadata`, `append_hard_links` a hard link group, and
//! `write_tree` a whole `hadris_fs::tree::Tree`. `write` writes a tree and
//! the trailer in one call and returns a [`Report`], whose warnings list the
//! metadata cpio cannot store.
//!
//! The on-disk headers are in [`raw`].
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

#![no_std]
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

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
mod entry;
mod error;
mod header;
mod options;

pub mod raw;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[cfg(feature = "alloc")]
    use hadris_fs::sync as fs;
    use hadris_io::sync as io;

    #[path = "read.rs"]
    mod read;
    pub use read::{CpioReader, Entry};
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{CpioWriter, write};
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    //! The asynchronous API, generated from the same source as `sync`.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    #[cfg(feature = "alloc")]
    use hadris_fs::r#async as fs;
    use hadris_io::r#async as io;

    #[path = "read.rs"]
    mod read;
    pub use read::{CpioReader, Entry};
    #[cfg(feature = "alloc")]
    #[path = "write.rs"]
    mod write;
    #[cfg(feature = "alloc")]
    pub use write::{CpioWriter, write};
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated a third time from the same source.
#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
pub mod async_send;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use entry::{NewEntry, Report};
pub use error::{Detail, Error};
pub use options::{CpioOptions, Format, ReaderOptions};

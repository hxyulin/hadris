//! # Hadris CPIO
//!
//! cpio archives, as Linux initramfs, RPM payloads and `cpio(1)` use them:
//! an allocation-free streaming reader and a streaming writer that also
//! writes a shared input tree.
//!
//! ## Reading
//!
//! `CpioReader` (in each mode: `sync::CpioReader` and `r#async::CpioReader`)
//! reads `newc` (`070701`), `newc` with
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
//! use hadris_fs::{Content, Node, Tree};
//! use hadris_io::{Cursor, StdIo};
//! use hadris_io::sync::Read;
//!
//! let mut tree = Tree::new();
//! tree.insert("etc/hostname", Node::file(Content::bytes("hadris\n")))?;
//! let mut out = StdIo::new(Vec::new());
//! let report = write(&mut out, &tree, &CpioOptions::default())?;
//! let archive = out.into_inner();
//! assert_eq!(report.size(), archive.len() as u64);
//! assert_eq!(hadris_cpio::plan(&tree, &CpioOptions::default())?, report);
//!
//! let mut reader = CpioReader::new(Cursor::new(&archive));
//! let mut names = Vec::new();
//! while let Some(mut entry) = reader.next_entry()? {
//!     names.push(entry.path_str()?.to_string());
//!     if entry.path() == b"etc/hostname" {
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
//! `next_segment()` reads archives concatenated after a trailer.
//!
//! ## Writing
//!
//! `Writer` (with `alloc`) streams entries to a `Write` stream in `newc`,
//! `newc` with checksums or `odc` ([`Format`]): `append` writes one
//! `hadris_fs::Node` (file, directory, symlink, device node, FIFO or
//! socket), `append_hard_links` a hard link group, and `append_file` a file
//! whose data is produced while writing. `finish` writes the trailer and
//! returns the stream with a `hadris_fs::Report`, so segments can be
//! concatenated.
//!
//! `write` writes a whole `hadris_fs::Tree` and the trailer, and [`plan`]
//! returns the same report without I/O: the archive size, where each
//! file's data starts, and the metadata cpio cannot store.
//! `read_tree` reads an archive back into a `Tree`.
//!
//! The on-disk headers are in [`raw`].
//!
//! Reading fails with [`hadris_fs::Error`]; [`Detail::of`] names the
//! field at fault, and an archive whose first header has no cpio magic
//! fails with
//! [`ErrorKind::NotRecognized`](hadris_fs::ErrorKind::NotRecognized).
//! Writing fails with [`hadris_fs::PathError`], which carries the path of
//! the entry that failed; [`Detail::from_code`] reads its detail.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
//! | `alloc` | via `std` | The writer, `plan` and `read_tree` |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API with `Send` futures in `r#async` |
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
mod build;
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
    pub use write::{EntryWriter, Writer, write};
    #[cfg(feature = "alloc")]
    #[path = "read_tree.rs"]
    mod read_tree;
    #[cfg(feature = "alloc")]
    pub use read_tree::read_tree;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
pub mod r#async;

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use build::plan;
pub use error::Detail;
pub use options::{CpioOptions, Format, ReaderOptions};

//! MBR, GPT and hybrid partition tables on `hadris-storage` block devices.
//!
//! [`Disk`] holds a partition table and the boot code of block 0. It does no
//! I/O: the mode modules ([`sync`], `r#async`, `async_send`) read it from a
//! [`BlockDevice`](hadris_storage::sync::BlockDevice), write it back, and
//! open a partition as a [`Slice`](hadris_storage::sync::Slice) of the
//! device. The block size is always the device's.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_part::gpt::types;
//! use hadris_part::sync::{create, open, read};
//! use hadris_part::{DiskLayout, Guid, PartitionKind, PartitionSpec, PartitionTable, Size};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
//! let layout = DiskLayout::gpt(Guid::from_bytes([7; 16]))
//!     .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(16)).with_name("EFI"))
//!     .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining));
//! create(&mut dev, &layout)?;
//!
//! let disk = read(&mut dev)?;
//! assert!(matches!(disk.table(), PartitionTable::Gpt(_)));
//! for p in disk.partitions() {
//!     let name = p.name().map(|n| n.to_string()).unwrap_or_default();
//!     println!("{} {} {} {:?} {name}", p.index(), p.start(), p.size_bytes(), p.kind());
//! }
//! let esp = disk.partition(0).unwrap();
//! assert_eq!(esp.kind(), PartitionKind::Gpt(types::EFI_SYSTEM));
//! let esp_dev = open(&mut dev, &esp)?;
//! # let _ = esp_dev;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! # Tables
//!
//! - [`Mbr`]: four primary slots and logical partitions in a chain of
//!   extended boot records.
//! - [`Gpt`]: GUID partition table. Its fields are private; edits keep it
//!   consistent and CRCs are computed whenever it is written. A read falls
//!   back to the backup copy when the primary is damaged.
//! - [`Hybrid`]: a GPT whose MBR mirrors up to three of its partitions,
//!   built from a [`HybridMbr`].
//!
//! Every edit (`add`, `add_logical`, `remove`, `resize`, `set_*`) checks
//! bounds and overlap and leaves the table unchanged when it fails.
//! [`DiskLayout`] places a list of [`PartitionSpec`]s on a disk in one step.
//!
//! # Without an allocator
//!
//! `scan` in each mode lists the partitions of a device through a callback,
//! with the same validation as `read`, and `open` needs no allocator either.
//! The on-disk layouts in [`raw`] and the GPT CRC ([`raw::crc32`]) are
//! always available. [`Disk`], the tables and the layout builder need
//! `alloc`.
//!
//! # Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `Guid::random` and `std::io::Error` conversions |
//! | `alloc` | via `std` | `Disk`, the tables, `DiskLayout`, and `read`, `write` and `create` |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API in `r#async` |
//! | `async-send` | No | The asynchronous API with `Send` futures in `async_send` |
//!
//! No feature changes what an item does. CRCs are always computed and
//! checked, and no GUID is ever made at random unless you call
//! [`Guid::random`].

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
// Sync and async APIs intentionally compile the same source modules twice.
#![allow(clippy::duplicate_mod)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Helpers shared by the tables and the I/O modes go unused in builds that
// have only one of them.
#![cfg_attr(
    not(all(feature = "alloc", any(feature = "sync", feature = "async"))),
    allow(dead_code)
)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod codec;
#[cfg(feature = "alloc")]
mod disk;
mod error;
#[cfg(feature = "alloc")]
mod gpt_table;
mod guid;
mod hybrid;
#[cfg(feature = "alloc")]
mod layout;
#[cfg(feature = "alloc")]
mod mbr;
mod mbr_type;
mod name;
mod partition;

pub mod gpt;
pub mod raw;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API: `read`, `write`, `create`, `scan` and `open`.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_storage::sync as storage;

    #[path = "io.rs"]
    mod io;
    #[cfg(feature = "alloc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub use io::{create, read, write};
    pub use io::{open, scan};
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    //! The asynchronous API, generated from the same source as `sync`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_storage::r#async as storage;

    #[path = "io.rs"]
    mod io;
    #[cfg(feature = "alloc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub use io::{create, read, write};
    pub use io::{open, scan};
}

#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
#[path = ""]
pub mod async_send {
    //! The asynchronous API over the `Send` devices of
    //! `hadris_storage::async_send`, whose futures are `Send` when the device
    //! and callbacks are. Generated from the same source as `r#async`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_storage::async_send as storage;

    #[path = "io.rs"]
    mod io;
    #[cfg(feature = "alloc")]
    #[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
    pub use io::{create, read, write};
    pub use io::{open, scan};
}

#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use disk::{Disk, PartitionTable, Partitions, Run, Runs};
pub use error::{Detail, Error, TableError};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use gpt_table::{Gpt, GptEntry};
pub use guid::{Guid, GuidParseError};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use hybrid::Hybrid;
pub use hybrid::HybridMbr;
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use layout::{Alignment, DiskLayout, PartitionSpec, Size};
#[cfg(feature = "alloc")]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub use mbr::{Mbr, MbrEntry};
pub use mbr_type::MbrType;
pub use name::PartitionName;
pub use partition::{GptCopy, Partition, PartitionFlags, PartitionKind, TableKind};

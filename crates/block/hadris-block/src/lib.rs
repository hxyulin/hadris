//! # Hadris Block
//!
//! Detection and opening of block volumes: FAT12, FAT16, FAT32 and NTFS on
//! any `hadris_storage` block device, next to the format crates it builds
//! on.
//!
//! [`detect`] reads the boot sector of a device and names its format, or
//! the partition table it holds. `OpenVolume`, in each mode
//! (`sync::OpenVolume`, `r#async::OpenVolume`, `async_send::OpenVolume`),
//! detects and mounts in one step and implements the `hadris_fs` `FsDriver`
//! trait by delegating to the driver it opened, so one generic function
//! lists any volume. A failed open gives the device back in an
//! [`OpenError`]. Neither needs an allocator.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_block::detect::{BlockFormat, FatVariant};
//! use hadris_block::sync::OpenVolume;
//! use hadris_fs::sync::DriverExt;
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 2 << 20], BlockSize::new(512).unwrap());
//! let dev = hadris_fat::sync::format(dev, hadris_fat::FormatOptions::new())?.into_inner();
//!
//! let mut volume = OpenVolume::open(dev)?;
//! assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::Fat12));
//! volume.write_file("/hello.txt", b"hi")?;
//! assert_eq!(volume.read_to_vec("/hello.txt")?, b"hi");
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write")))]
//! # fn main() {}
//! ```
//!
//! A partitioned disk is opened one partition at a time: `hadris_part`
//! (re-exported as [`part`] with the `part` feature) reads the table and
//! turns a partition into a `hadris_storage` `Slice`, which `OpenVolume`
//! opens. exFAT is detected but not opened while it is a preview in
//! `hadris-fat`.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and `std::fs::File` devices |
//! | `alloc` | via `std` | `AnyError` conversions |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API in `r#async` |
//! | `async-send` | No | The asynchronous API with `Send` futures in `async_send` |
//! | `write` | No | `hadris_fat` formatting, through [`fat`] |
//! | `part` | No | Re-exports `hadris-part` as [`part`] |
//! | `unstable-ntfs` | No | Re-exports the `hadris-ntfs` preview as `ntfs` and reaches `OpenVolume`'s NTFS driver |
//!
//! No feature changes what an item does: `OpenVolume` always opens FAT and
//! NTFS.

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod detect;
mod error;

pub use error::{Detail, Error, OpenError};

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }

    macro_rules! impl_block_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    use crate::detect::sync::detect;
    use hadris_fat::sync::FatFs;
    use hadris_ntfs::sync::NtfsFs;
    use hadris_storage::sync::BlockDevice;

    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    //! The asynchronous API, generated from the same source as `sync`.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! impl_block_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    use crate::detect::r#async::detect;
    use hadris_fat::r#async::FatFs;
    use hadris_ntfs::r#async::NtfsFs;
    use hadris_storage::r#async::BlockDevice;

    #[allow(clippy::duplicate_mod)]
    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated a third time from the same source.
#[cfg(feature = "async-send")]
#[cfg_attr(docsrs, doc(cfg(feature = "async-send")))]
#[path = ""]
pub mod async_send {
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    macro_rules! impl_block_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async_send, $($t)*); };
    }

    use crate::detect::async_send::detect;
    use hadris_fat::async_send::FatFs;
    use hadris_ntfs::async_send::NtfsFs;
    use hadris_storage::async_send::BlockDevice;

    #[allow(clippy::duplicate_mod)]
    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

/// Block devices and adapters.
pub use hadris_storage as storage;

/// FAT12, FAT16 and FAT32, which `OpenVolume` opens as `FatFs`.
pub use hadris_fat as fat;

/// MBR, GPT and hybrid partition tables. `part::sync::open` (and its
/// `r#async` and `async_send` forms) turns a partition into a
/// `hadris-storage` `Slice` of the disk, which `OpenVolume` opens.
#[cfg(feature = "part")]
#[cfg_attr(docsrs, doc(cfg(feature = "part")))]
pub use hadris_part as part;

/// The NTFS preview, which `OpenVolume` opens as `NtfsFs`. Its native API
/// may change in 3.x minors.
#[cfg(feature = "unstable-ntfs")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
pub use hadris_ntfs as ntfs;

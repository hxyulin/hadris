//! # Hadris Block
//!
//! Detection and opening of block volumes: FAT12, FAT16, FAT32, exFAT and NTFS on
//! any `hadris_storage` block device, next to the format crates it builds
//! on.
//!
//! [`detect`] reads the boot sector of a device and names its format, or
//! the partition table it holds. `OpenVolume`, in each mode
//! (`sync::OpenVolume`, `r#async::OpenVolume`),
//! detects and mounts in one step and implements the `hadris_fs`
//! `FileSystem` trait by delegating to the driver it opened, so one generic function
//! lists any volume. A failed open gives the device back in a
//! `hadris_fs::MountError`. Detection needs no allocator; `OpenVolume` needs
//! `alloc`, as the FAT and exFAT drivers do.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_block::detect::{BlockFormat, FatVariant};
//! use hadris_block::sync::OpenVolume;
//! use hadris_fs::OpenOptions;
//! use hadris_fs::sync::Volume;
//! use hadris_storage::{BlockSize, MemDevice};
//! use std::io::{Read, Write};
//!
//! let mut dev = MemDevice::new(vec![0u8; 2 << 20], BlockSize::new(512).unwrap());
//! hadris_fat::sync::format(&mut dev, &hadris_fat::FatOptions::new())?;
//!
//! let volume = OpenVolume::open(dev)?;
//! assert_eq!(volume.format(), BlockFormat::Fat(FatVariant::Fat12));
//! let vol = Volume::new(volume);
//! let mut file = vol.open("/hello.txt", OpenOptions::new().write().create())?;
//! file.write_all(b"hi")?;
//! file.close()?;
//! let mut text = String::new();
//! vol.open("/hello.txt", OpenOptions::new().read())?.read_to_string(&mut text)?;
//! assert_eq!(text, "hi");
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write")))]
//! # fn main() {}
//! ```
//!
//! A partitioned disk is opened one partition at a time: `hadris_part`
//! (re-exported as `part` with the `part` feature) reads the table and
//! turns a partition into a `hadris_storage` `Partition`, which `OpenVolume`
//! opens.
//!
//! ## Features
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | Yes | Implies `alloc`; `std::io::Error` conversions and `hadris_storage::host::FileDevice` |
//! | `alloc` | via `std` | `OpenVolume` and `PathError` conversions |
//! | `sync` | Yes | The blocking API in `sync` |
//! | `async` | No | The asynchronous API with `Send` futures in `r#async` |
//! | `write` | No | `hadris_fat` formatting, through [`fat`] |
//! | `part` | No | Re-exports `hadris-part` as `part` |
//! | `unstable-ntfs` | No | Re-exports the `hadris-ntfs` preview as `ntfs` and reaches `OpenVolume`'s NTFS driver |
//!
//! No feature changes what an item does: `OpenVolume` always opens FAT,
//! exFAT and NTFS.

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

pub use error::Detail;

#[cfg(feature = "sync")]
#[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
#[path = ""]
pub mod sync {
    //! The blocking API.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }

    #[cfg(feature = "alloc")]
    use crate::detect::sync::detect;
    #[cfg(feature = "alloc")]
    use hadris_fat::exfat::sync::ExFatFs;
    #[cfg(feature = "alloc")]
    use hadris_fat::sync::FatFs;
    #[cfg(feature = "alloc")]
    use hadris_fs::sync::FileSystem;
    #[cfg(feature = "alloc")]
    use hadris_ntfs::sync::NtfsFs;
    #[cfg(feature = "alloc")]
    use hadris_storage::sync::BlockDevice;

    #[cfg(feature = "alloc")]
    #[path = "volume.rs"]
    mod volume;
    #[cfg(feature = "alloc")]
    pub use volume::OpenVolume;
}

/// The asynchronous API with `Send` futures, for generic code on
/// multi-threaded executors, generated from the same source as `sync`.
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[path = ""]
pub mod r#async {
    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    #[cfg(feature = "alloc")]
    use crate::detect::r#async::detect;
    #[cfg(feature = "alloc")]
    use hadris_fat::r#async::FatFs;
    #[cfg(feature = "alloc")]
    use hadris_fat::exfat::r#async::ExFatFs;
    #[cfg(feature = "alloc")]
    use hadris_fs::r#async::FileSystem;
    #[cfg(feature = "alloc")]
    use hadris_ntfs::r#async::NtfsFs;
    #[cfg(feature = "alloc")]
    use hadris_storage::r#async::BlockDevice;

    #[cfg(feature = "alloc")]
    #[allow(clippy::duplicate_mod)]
    #[path = "volume.rs"]
    mod volume;
    #[cfg(feature = "alloc")]
    pub use volume::OpenVolume;
}

/// Block devices and adapters.
pub use hadris_storage as storage;

/// FAT12, FAT16 and FAT32, which `OpenVolume` opens as `FatFs`, and exFAT,
/// which it opens as `fat::exfat` `ExFatFs`.
pub use hadris_fat as fat;

/// MBR, GPT and hybrid partition tables. `part::sync::open` (and its
/// `r#async` forms) turns a partition into a
/// `hadris-storage` `Partition` of the disk, which `OpenVolume` opens.
#[cfg(feature = "part")]
#[cfg_attr(docsrs, doc(cfg(feature = "part")))]
pub use hadris_part as part;

/// The NTFS preview, which `OpenVolume` opens as `NtfsFs`. Its native API
/// may change in 3.x minors.
#[cfg(feature = "unstable-ntfs")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-ntfs")))]
pub use hadris_ntfs as ntfs;

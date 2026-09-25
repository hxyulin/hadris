//! The exFAT driver, `ExFatFs`, its formatter and checker.
//!
//! It follows the same shape as `FatFs`. `ExFatFs` is generated for each mode
//! (`exfat::sync`, `exfat::r#async`) with `check`
//! and, with `write`, `format` and the tree writer `write`; the
//! mode-independent types are here. It
//! needs `alloc` and implements the `hadris_fs` `FileSystem` trait, so
//! `Volume` and its handles work on it.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::exfat::ExFatOptions;
//! use hadris_fat::exfat::sync::{check, format};
//! use hadris_fs::sync::Volume;
//! use hadris_fs::{MountOptions, OpenOptions};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let mut dev = MemDevice::new(vec![0u8; 16 << 20], BlockSize::new(512).unwrap());
//! format(&mut dev, &ExFatOptions::new())?;
//! assert!(check(&mut dev, &mut [0u8; 4096], |_| {})?.is_clean());
//! let vol = Volume::new(hadris_fat::exfat::sync::ExFatFs::mount(dev, MountOptions::new())?);
//! vol.create_dir_all("/Photos")?;
//! let mut file = vol.open("/Photos/Été.txt", OpenOptions::new().write().create())?;
//! file.write(b"hello")?;
//! file.close()?;
//! assert_eq!(vol.metadata("/photos/ÉTÉ.TXT")?.len(), 5);
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sync", feature = "write", feature = "std")))]
//! # fn main() {}
//! ```
//!
//! The driver reads contiguous (`NoFatChain`) and chained allocations,
//! fragmented allocation bitmaps and up-case tables, entry sets that cross
//! clusters and benign secondary entries, which it keeps across renames. It
//! writes FAT chains, grows directories, and keeps `VolumeDirty` and
//! `PercentInUse` as the specification recommends. On TexFAT volumes, which
//! have two FATs and two allocation bitmaps, it follows `ActiveFat` and
//! keeps both copies equal; TexFAT transactions and repair are not
//! supported.

mod options;
pub use hadris_fat_raw::exfat::{Detail, Geometry};

/// Reads a little-endian `u16` at `at`.
#[cfg_attr(
    not(all(feature = "alloc", any(feature = "sync", feature = "async"))),
    allow(dead_code)
)]
pub(crate) fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Reads a little-endian `u32` at `at`.
#[cfg_attr(
    not(all(feature = "alloc", any(feature = "sync", feature = "async"))),
    allow(dead_code)
)]
pub(crate) fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads a little-endian `u64` at `at`.
#[cfg_attr(
    not(all(feature = "alloc", any(feature = "sync", feature = "async"))),
    allow(dead_code)
)]
pub(crate) fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}
#[cfg(feature = "write")]
pub use options::ExFatOptions;
pub use options::VolumeLabel;

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous exFAT API.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    #[cfg(any(feature = "alloc", feature = "write"))]
    use crate::sync::block_io;
    #[cfg(feature = "write")]
    use crate::sync::mkfs as fatmkfs;
    use hadris_fat_raw::exfat::io::sync as exio;
    #[cfg(any(feature = "alloc", feature = "write"))]
    use hadris_storage::sync as storage;

    #[cfg(feature = "alloc")]
    use hadris_fs::sync as fsapi;

    #[cfg(feature = "alloc")]
    #[path = "fs.rs"]
    mod fs;
    pub use exio::check;
    #[cfg(feature = "alloc")]
    pub use fs::ExFatFs;
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
    #[cfg(all(feature = "alloc", feature = "write"))]
    pub use mkfs::write;
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous exFAT API with `Send` futures. Its futures are
    //! `Send` when the device is.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    #[cfg(any(feature = "alloc", feature = "write"))]
    use crate::r#async::block_io;
    #[cfg(feature = "write")]
    use crate::r#async::mkfs as fatmkfs;
    use hadris_fat_raw::exfat::io::r#async as exio;
    #[cfg(any(feature = "alloc", feature = "write"))]
    use hadris_storage::r#async as storage;

    #[cfg(feature = "alloc")]
    use hadris_fs::r#async as fsapi;

    #[cfg(feature = "alloc")]
    #[path = "fs.rs"]
    mod fs;
    pub use exio::check;
    #[cfg(feature = "alloc")]
    pub use fs::ExFatFs;
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
    #[cfg(all(feature = "alloc", feature = "write"))]
    pub use mkfs::write;
}

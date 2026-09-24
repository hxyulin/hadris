//! The exFAT driver, `ExFatFs`, its formatter and checker.
//!
//! It follows the same shape as `FatFs`. `ExFatFs` is generated for each mode
//! (`exfat::sync`, `exfat::r#async`) with `check`
//! and, with `write`, `format`; the mode-independent types are here. It needs no allocator and implements the `hadris_fs` `FsDriver`
//! trait, so `Volume` and the path helpers work on it.
//!
//! ```rust
//! # #[cfg(all(feature = "sync", feature = "write", feature = "std"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use hadris_fat::exfat::FormatOptions;
//! use hadris_fat::exfat::sync::{check, format};
//! use hadris_fs::sync::{PathExt, Volume};
//! use hadris_storage::{BlockSize, MemDevice};
//!
//! let dev = MemDevice::new(vec![0u8; 16 << 20], BlockSize::new(512).unwrap());
//! let mut dev = format(dev, FormatOptions::new())?.into_inner();
//! assert!(check(&mut dev, &mut [0u8; 4096], |_| {})?.is_clean());
//! let vol = Volume::new(hadris_fat::exfat::sync::ExFatFs::open(dev)?);
//! vol.create_dir_all("/Photos")?;
//! vol.write_file("/Photos/Été.txt", b"hello")?;
//! assert_eq!(vol.read_to_vec("/photos/ÉTÉ.TXT")?, b"hello");
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
/// The exFAT part of the on-disk layer, `hadris_fat_raw::exfat`: the boot
/// sector and directory entry layouts with their constants, and the codecs.
///
/// The layouts mirror the exFAT specification and stay exhaustive.
pub use hadris_fat_raw::exfat as raw;

pub use hadris_fat_raw::exfat::Detail;

/// Reads a little-endian `u16` at `at`.
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Reads a little-endian `u32` at `at`.
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads a little-endian `u64` at `at`.
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub(crate) fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}
#[cfg(feature = "write")]
pub use options::FormatOptions;
pub use options::{MountOptions, VolumeLabel};

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous exFAT API.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use crate::sync::block_io;
    use hadris_fat_raw::exfat::io::sync as exio;
    use hadris_storage::sync as storage;

    macro_rules! impl_exfat_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    #[path = "fs.rs"]
    mod fs;
    pub use exio::check;
    pub use fs::ExFatFs;
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous exFAT API with `Send` futures. Its futures are
    //! `Send` when the device is and the node table holds `Send` values.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    use crate::r#async::block_io;
    use hadris_fat_raw::exfat::io::r#async as exio;
    use hadris_storage::r#async as storage;

    macro_rules! impl_exfat_driver {
        (impl[D: BlockDevice, T: NodeTable, C: Clock] $($rest:tt)*) => {
            hadris_fs::impl_fs_driver!(
                async,
                impl[D: BlockDevice, T: NodeTable<With<Node>: Send>, C: Clock + Send] $($rest)*
            );
        };
    }

    #[path = "fs.rs"]
    mod fs;
    pub use exio::check;
    pub use fs::ExFatFs;
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

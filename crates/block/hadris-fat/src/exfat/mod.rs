//! Unstable exFAT preview: the `ExFatFs` driver, its formatter and checker.
//!
//! This module is enabled by `unstable-exfat` and is outside the Hadris API
//! stability promise: its items may change in minor releases. It follows
//! the same shape as `FatFs`. `ExFatFs` is generated for each mode
//! (`exfat::sync`, `exfat::r#async`, `exfat::async_send`) with `check`,
//! `check_with` and, with `write`, `format`; the mode-independent types are
//! here. It needs no allocator and implements the `hadris_fs` `FsDriver`
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
//! let mut fs = format(dev, FormatOptions::new())?;
//! assert!(check(&mut fs)?.is_clean());
//! let vol = Volume::new(fs);
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

#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
mod codec;
mod findings;
mod options;
/// Raw on-disk exFAT boot sector and directory entry layouts.
///
/// The items mirror the exFAT specification and stay exhaustive.
pub mod raw;

pub use findings::{CheckReport, Finding, FindingKind};
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
    use hadris_storage::sync as storage;

    macro_rules! impl_exfat_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(sync, $($t)*); };
    }

    #[path = "fs.rs"]
    mod fs;
    pub use fs::{ExFatFs, check, check_with};
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous exFAT API.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use crate::r#async::block_io;
    use hadris_storage::r#async as storage;

    macro_rules! impl_exfat_driver {
        ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async, $($t)*); };
    }

    #[path = "fs.rs"]
    mod fs;
    pub use fs::{ExFatFs, check, check_with};
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

#[cfg(feature = "async-send")]
#[path = ""]
pub mod async_send {
    //! The asynchronous exFAT API with `Send` futures. Its futures are
    //! `Send` when the device is and the node table holds `Send` values.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    use crate::async_send::block_io;
    use hadris_storage::async_send as storage;

    macro_rules! impl_exfat_driver {
        (impl[D: BlockDevice, T: NodeTable, C: Clock] $($rest:tt)*) => {
            hadris_fs::impl_fs_driver!(
                async_send,
                impl[D: BlockDevice, T: NodeTable<With<Node>: Send>, C: Clock + Send] $($rest)*
            );
        };
    }

    #[path = "fs.rs"]
    mod fs;
    pub use fs::{ExFatFs, check, check_with};
    #[cfg(feature = "write")]
    #[path = "mkfs.rs"]
    mod mkfs;
    #[cfg(feature = "write")]
    pub use mkfs::format;
}

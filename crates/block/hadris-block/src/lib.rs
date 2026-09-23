//! Block devices, partition tables, and FAT filesystems for the Hadris Rust
//! storage stack.
//!
//! This `no_std`-compatible facade groups format-neutral sector and block-device
//! interfaces, GPT and MBR partition tables, filesystem detection, and
//! FAT12/16/32 without hiding their concrete APIs.

#![no_std]
#![deny(missing_docs)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "detect")]
/// Lightweight, non-destructive block-format detection.
pub mod detect;
#[cfg(all(feature = "detect", feature = "fat"))]
mod error;

#[cfg(all(feature = "detect", feature = "fat"))]
pub use error::{Error, OpenError, Result};

#[cfg(all(feature = "detect", feature = "fat", feature = "sync"))]
#[path = ""]
/// Synchronous detection and unified volume opening.
pub mod sync {
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }

    use crate::detect::sync::detect;
    use hadris_fat::sync::FatFs;
    use hadris_storage::sync::BlockDevice;

    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

#[cfg(all(feature = "detect", feature = "fat", feature = "async"))]
#[path = ""]
/// Asynchronous detection and unified volume opening.
pub mod r#async {
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use crate::detect::r#async::detect;
    use hadris_fat::r#async::FatFs;
    use hadris_storage::r#async::BlockDevice;

    #[allow(clippy::duplicate_mod)]
    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

#[cfg(all(feature = "detect", feature = "fat", feature = "async-send"))]
#[path = ""]
/// Asynchronous detection and unified volume opening whose futures are
/// `Send`, generated from the same source as `r#async`, over the
/// `async_send` modules of `hadris-storage` and `hadris-fat`.
pub mod async_send {
    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use crate::detect::async_send::detect;
    use hadris_fat::async_send::FatFs;
    use hadris_storage::async_send::BlockDevice;

    #[allow(clippy::duplicate_mod)]
    #[path = "volume.rs"]
    mod volume;
    pub use volume::OpenVolume;
}

/// Format-neutral block geometry and device capabilities.
#[cfg(feature = "storage")]
pub use hadris_storage as storage;

/// FAT12/16/32 filesystem support and exFAT format detection.
///
/// The unified volume opener mounts FAT12/16/32 as `hadris_fat` `FatFs`
/// drivers. The leaf `hadris-fat` crate carries a separate unstable exFAT
/// preview.
#[cfg(feature = "fat")]
pub use hadris_fat as fat;

/// MBR, GPT, and hybrid partition-table support. `part::sync::open` (and
/// its `r#async` and `async_send` forms) turns a partition into a
/// `hadris-storage` `Slice` of the disk, which `OpenVolume` mounts.
#[cfg(feature = "part")]
pub use hadris_part as part;

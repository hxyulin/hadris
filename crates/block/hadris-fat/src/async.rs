#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => {};
}
#[allow(unused_macros)]
macro_rules! async_only { ($($item:tt)*) => { $($item)* }; }

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fat_raw::io::async_ as rawio;
#[cfg(feature = "alloc")]
use hadris_fs::local as fsapi;
#[cfg(any(feature = "alloc", feature = "write"))]
use hadris_storage::async_ as storage;

#[cfg(any(feature = "alloc", feature = "write"))]
#[path = "block_io.rs"]
pub(crate) mod block_io;
#[cfg(feature = "alloc")]
#[path = "fatfs.rs"]
mod fatfs;
#[cfg(feature = "alloc")]
pub use fatfs::FatFs;
pub use rawio::check;
#[cfg(feature = "write")]
#[path = "mkfs.rs"]
pub(crate) mod mkfs;
#[cfg(feature = "write")]
pub use mkfs::format;
#[cfg(all(feature = "alloc", feature = "write"))]
pub use mkfs::write;

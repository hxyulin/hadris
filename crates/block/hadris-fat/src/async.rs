#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

use hadris_fat_raw::io::r#async as rawio;
#[cfg(feature = "alloc")]
use hadris_fs::r#async as fsapi;
#[cfg(any(feature = "alloc", feature = "write"))]
use hadris_storage::r#async as storage;

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

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

use hadris_fat_raw::io::r#async as rawio;
use hadris_fs::r#async as fsapi;
use hadris_io::r#async as io;
use hadris_storage::r#async as storage;

#[path = "block_io.rs"]
pub(crate) mod block_io;
#[path = "fatfs.rs"]
mod fatfs;
pub use fatfs::FatFs;
pub use rawio::check;
#[cfg(feature = "write")]
#[path = "mkfs.rs"]
mod mkfs;
#[cfg(feature = "write")]
pub use mkfs::format;

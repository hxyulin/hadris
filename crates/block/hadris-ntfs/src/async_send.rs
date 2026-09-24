#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_storage::async_send as storage;

macro_rules! impl_ntfs_driver {
    ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async_send, $($t)*); };
}

#[path = "fs.rs"]
mod fs;
pub use fs::NtfsFs;

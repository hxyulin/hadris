#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

use hadris_storage::async_send as storage;

macro_rules! impl_fat_driver {
    (impl[D: BlockDevice, T: NodeTable] $($rest:tt)*) => {
        hadris_fs::impl_fs_driver!(async_send, impl[D: BlockDevice, T: NodeTable<With<Node>: Send>] $($rest)*);
    };
}

#[path = "fatfs.rs"]
mod fatfs;
pub use fatfs::FatFs;

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

use hadris_storage::async_send as storage;

macro_rules! impl_fat_driver {
    (impl[D: BlockDevice, T: NodeTable, C: Clock, P: CodePage] $($rest:tt)*) => {
        hadris_fs::impl_fs_driver!(
            async_send,
            impl[D: BlockDevice, T: NodeTable<With<Node>: Send>, C: Clock + Send, P: CodePage + Send] $($rest)*
        );
    };
}

#[path = "fatfs.rs"]
mod fatfs;
pub use fatfs::FatFs;
#[cfg(feature = "write")]
#[path = "mkfs.rs"]
mod mkfs;
#[cfg(feature = "write")]
pub use mkfs::format;

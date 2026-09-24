#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

use hadris_fat_raw::io::r#async as rawio;
use hadris_storage::r#async as storage;

macro_rules! impl_fat_driver {
    (impl[D: BlockDevice, T: NodeTable, C: Clock, P: CodePage] $($rest:tt)*) => {
        hadris_fs::impl_fs_driver!(
            async,
            impl[D: BlockDevice, T: NodeTable<With<Node>: Send>, C: Clock + Send, P: CodePage + Send] $($rest)*
        );
    };
}

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

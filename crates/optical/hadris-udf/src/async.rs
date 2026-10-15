#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::local as fs;
#[cfg(feature = "alloc")]
use hadris_iso::async_ as iso;
use hadris_storage::async_ as storage;

use hadris_fs::local::FileSystem;

#[path = "read.rs"]
mod read;
pub use read::UdfFs;
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{write, write_bridge};

hadris_fs::__forward_filesystem! {
    shims UdfFs<D: storage::BlockDevice> { read resolve write }
}

hadris_fs::__forward_filesystem! {
    #[cfg(feature = "async")]
    send UdfFs<D: hadris_storage::async_::SendBlockDevice> as hadris_fs::async_::FileSystem { read }
}

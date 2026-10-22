#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "write")]
use hadris_fs::async_ as fs;
#[cfg(feature = "bridge")]
use hadris_iso::async_ as iso;
use hadris_storage::async_ as storage;

use hadris_fs::async_::FileSystem;

#[path = "read.rs"]
mod read;
pub use read::UdfFs;
#[cfg(feature = "write")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "write")]
pub use write::write;
#[cfg(feature = "bridge")]
pub use write::write_bridge;

hadris_fs::__forward_filesystem! {
    shims UdfFs<D: storage::BlockDevice> { read resolve write }
}

hadris_fs::__forward_filesystem! {
    #[cfg(feature = "async")]
    send UdfFs<D: hadris_storage::async_::SendBlockDevice> as hadris_fs::async_::SendFileSystem { read resolve write }
}

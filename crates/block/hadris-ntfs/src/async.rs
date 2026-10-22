macro_rules! send_filesystem { ($($item:tt)*) => { $($item)* }; }
#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::async_::FileSystem;
use hadris_storage::async_ as storage;

#[path = "fs.rs"]
mod fs;
pub use fs::NtfsFs;

hadris_fs::__forward_filesystem! {
    shims NtfsFs<D: storage::BlockDevice> { resolve write }
}

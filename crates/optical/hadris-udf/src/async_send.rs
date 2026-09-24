#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::async_send as fs;
use hadris_storage::async_send as storage;

macro_rules! impl_udf_driver {
    ($($t:tt)*) => { hadris_fs::impl_fs_driver!(async_send, $($t)*); };
}

#[path = "read.rs"]
mod read;
pub use read::UdfFs;
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{plan, write};

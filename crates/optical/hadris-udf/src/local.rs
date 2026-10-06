#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::local::FileSystem;
use hadris_storage::local as storage;

#[path = "read.rs"]
mod reader;
pub use reader::UdfFs;

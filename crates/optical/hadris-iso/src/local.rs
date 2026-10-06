#[allow(unused_macros)]
macro_rules! sessions_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::local::FileSystem;
use hadris_storage::local as storage;

#[path = "image.rs"]
mod reader;
pub use reader::IsoFs;

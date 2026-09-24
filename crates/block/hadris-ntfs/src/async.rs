#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::r#async::FileSystem;
use hadris_storage::r#async as storage;

#[path = "fs.rs"]
mod fs;
pub use fs::NtfsFs;

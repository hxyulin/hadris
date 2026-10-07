macro_rules! send_filesystem { ($($item:tt)*) => { $($item)* }; }
#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::local::FileSystem;
use hadris_storage::async_ as storage;

#[path = "fs.rs"]
mod fs;
pub use fs::NtfsFs;

#[path = "async_defaults.rs"]
mod defaults;

#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::r#async as fs;
use hadris_storage::r#async as storage;

use hadris_fs::r#async::FileSystem;

#[path = "read.rs"]
mod read;
pub use read::UdfFs;
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{plan, write};

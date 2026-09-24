#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_iso::async_send as iso;
use hadris_storage::async_send as storage;
use hadris_udf::async_send as udf;

#[path = "write.rs"]
mod write;
pub use write::{plan, write};

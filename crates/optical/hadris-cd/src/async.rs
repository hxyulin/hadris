#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_storage::r#async as storage;
use hadris_udf::r#async as udf;

#[path = "write.rs"]
mod write;
pub use write::write;

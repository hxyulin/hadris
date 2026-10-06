use hadris_io::local::{Read, Seek, Write};

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_io::local::MaybeSend;

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

use hadris_io::r#async::{Read, Seek, Write};

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_io::r#async::MaybeSend;

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

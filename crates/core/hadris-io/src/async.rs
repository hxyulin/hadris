use embedded_io_async as base;

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

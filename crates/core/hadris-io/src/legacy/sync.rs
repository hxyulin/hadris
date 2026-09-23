use embedded_io as base;

macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

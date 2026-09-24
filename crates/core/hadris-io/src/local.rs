#[cfg(feature = "embedded-io")]
use embedded_io_async as base;

macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

macro_rules! local_only {
    ($($item:tt)*) => { $($item)* };
}

/// Implemented by every type in this mode; `Send` in the `async_send` mode.
pub trait MaybeSend {}
impl<T: ?Sized> MaybeSend for T {}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

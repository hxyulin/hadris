macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

macro_rules! local_only {
    ($($item:tt)*) => {};
}

/// `Send` in this mode, so a bound on it proves a future `Send`.
pub trait MaybeSend: Send {}
impl<T: Send + ?Sized> MaybeSend for T {}

#[allow(clippy::duplicate_mod)]
#[path = "api.rs"]
mod api;
pub use api::*;

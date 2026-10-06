#[allow(unused_imports)]
use hadris_io::r#async as io;
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
}

#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! async_only {
    ($($item:tt)*) => { $($item)* };
}

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

/// The lock of an async `Volume`: `async_lock::Mutex`, safe to hold across
/// `.await`, which needs only `alloc`.
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
mod lock {
    pub(crate) type Guard<'a, T> = async_lock::MutexGuard<'a, T>;

    pub(crate) struct Lock<T>(async_lock::Mutex<T>);

    impl<T> Lock<T> {
        pub(crate) fn new(value: T) -> Self {
            Self(async_lock::Mutex::new(value))
        }

        pub(crate) async fn lock(&self) -> Guard<'_, T> {
            self.0.lock().await
        }

        pub(crate) fn try_lock(&self) -> Option<Guard<'_, T>> {
            self.0.try_lock()
        }

        pub(crate) fn into_inner(self) -> T {
            self.0.into_inner()
        }
    }
}

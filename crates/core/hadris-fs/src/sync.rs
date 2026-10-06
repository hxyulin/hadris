#[allow(unused_imports)]
use hadris_io::sync as io;
macro_rules! io_transform {
    ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
}

#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => { $($item)* };
}

#[allow(unused_macros)]
macro_rules! async_only {
    ($($item:tt)*) => {};
}

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

/// The lock of a sync `Volume`: `std::sync::Mutex`, with poisoning ignored,
/// since a panic inside a driver call leaves the driver as a failed call
/// does.
#[cfg(feature = "std")]
mod lock {
    pub(crate) type Guard<'a, T> = std::sync::MutexGuard<'a, T>;

    pub(crate) struct Lock<T>(std::sync::Mutex<T>);

    impl<T> Lock<T> {
        pub(crate) fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }

        pub(crate) fn lock(&self) -> Guard<'_, T> {
            self.0.lock().unwrap_or_else(|poison| poison.into_inner())
        }

        pub(crate) fn try_lock(&self) -> Option<Guard<'_, T>> {
            match self.0.try_lock() {
                Ok(guard) => Some(guard),
                Err(std::sync::TryLockError::Poisoned(poison)) => Some(poison.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => None,
            }
        }

        pub(crate) fn into_inner(self) -> T {
            self.0
                .into_inner()
                .unwrap_or_else(|poison| poison.into_inner())
        }
    }
}

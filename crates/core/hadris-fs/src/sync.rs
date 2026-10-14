#[allow(unused_macros)]
macro_rules! send_only {
    ($($item:tt)*) => { $($item)* };
}

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

/// Runs a `Volume` call; the sync lock detects panics by poisoning.
#[allow(unused_macros)]
macro_rules! watched {
    ($vol:expr, $body:block) => {
        $body
    };
}

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
pub use api::*;

/// The lock of a sync `Volume`: `std::sync::Mutex`, which a panic while
/// it is held poisons. A poisoned lock hands out no guard, so the volume
/// reports itself failed instead of reaching a driver left mid-call.
#[cfg(feature = "std")]
mod lock {
    pub(crate) type Guard<'a, T> = std::sync::MutexGuard<'a, T>;

    pub(crate) struct Lock<T>(std::sync::Mutex<T>);

    impl<T> Lock<T> {
        pub(crate) fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }

        /// The guard, or `None` when the lock is poisoned.
        pub(crate) fn lock(&self) -> Option<Guard<'_, T>> {
            self.0.lock().ok()
        }

        /// The guard, or `None` when the lock is held or poisoned.
        pub(crate) fn try_lock(&self) -> Option<Guard<'_, T>> {
            self.0.try_lock().ok()
        }

        pub(crate) fn poisoned(&self) -> bool {
            self.0.is_poisoned()
        }

        pub(crate) fn into_inner(self) -> T {
            self.0
                .into_inner()
                .unwrap_or_else(|poison| poison.into_inner())
        }
    }
}

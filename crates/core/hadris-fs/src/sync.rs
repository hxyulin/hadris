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
#[cfg(feature = "unstable-contract")]
#[doc(hidden)]
pub use api::contract;
#[cfg(feature = "alloc")]
pub use api::{ContentReader, copy_tree};
#[cfg(feature = "std")]
pub use api::{File, ReadDir, Volume, VolumeGuard, WalkDir, WalkDirEntry, read_tree};
pub use api::{FileSystem, Walk};

/// The lock of a sync `Volume`: `std::sync::Mutex`, which a panic while
/// it is held poisons. A poisoned lock hands out no guard, so the volume
/// reports itself failed instead of reaching a driver left mid-call.
#[cfg(feature = "std")]
mod lock {
    use std::cell::RefCell;
    use std::ops::{Deref, DerefMut};
    use std::vec::Vec;

    std::thread_local! {
        static HELD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) struct Guard<'a, T> {
        guard: std::sync::MutexGuard<'a, T>,
        lock: usize,
    }

    impl<T> Deref for Guard<'_, T> {
        type Target = T;

        fn deref(&self) -> &T {
            &self.guard
        }
    }

    impl<T> DerefMut for Guard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.guard
        }
    }

    impl<T> Drop for Guard<'_, T> {
        fn drop(&mut self) {
            let _ = HELD.try_with(|held| {
                let mut held = held.borrow_mut();
                if let Some(index) = held.iter().position(|&lock| lock == self.lock) {
                    held.swap_remove(index);
                }
            });
        }
    }

    pub(crate) struct Lock<T>(std::sync::Mutex<T>);

    impl<T> Lock<T> {
        pub(crate) fn new(value: T) -> Self {
            Self(std::sync::Mutex::new(value))
        }

        /// The guard, or `None` when the lock is poisoned.
        pub(crate) fn lock(&self) -> Option<Guard<'_, T>> {
            self.0.lock().ok().map(|guard| self.track(guard))
        }

        /// The guard, or `None` when the lock is held or poisoned.
        pub(crate) fn try_lock(&self) -> Option<Guard<'_, T>> {
            self.0.try_lock().ok().map(|guard| self.track(guard))
        }

        fn track<'a>(&self, guard: std::sync::MutexGuard<'a, T>) -> Guard<'a, T> {
            let lock = core::ptr::from_ref(self).addr();
            let _ = HELD.try_with(|held| held.borrow_mut().push(lock));
            Guard { guard, lock }
        }

        pub(crate) fn held_by_current_thread(&self) -> bool {
            let lock = core::ptr::from_ref(self).addr();
            HELD.try_with(|held| held.borrow().contains(&lock))
                .unwrap_or(false)
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

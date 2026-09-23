use core::future::Future;
use core::ops::DerefMut;

/// A mutual-exclusion cell for [`Volume`](super::Volume) whose futures and
/// guards are `Send`.
///
/// Written by hand rather than generated: the guard is an opaque type, not a
/// `Guard<'a>` associated type, because proving a future that holds a GAT
/// guard `Send` needs `Self: 'a` for every lifetime, which rustc rejects.
pub trait Lock<T>: Send + Sync {
    /// Wraps `value`.
    fn new(value: T) -> Self;

    /// Acquires the lock.
    fn lock(&self) -> impl Future<Output = impl DerefMut<Target = T> + Send + '_> + Send;

    /// Acquires the lock if it is free.
    fn try_lock(&self) -> Option<impl DerefMut<Target = T> + Send + '_>;

    /// Borrows the value without locking.
    fn get_mut(&mut self) -> &mut T;

    /// Returns the value.
    fn into_inner(self) -> T;
}

/// Selects a lock type for any `Send` value.
pub trait LockKind {
    /// The lock wrapping a `T`.
    type Lock<T: Send>: Lock<T>;
}

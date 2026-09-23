use core::ops::DerefMut;

io_transform! {

/// A mutual-exclusion cell for [`Volume`](super::Volume). In async builds
/// `lock` is awaited, so only locks that are safe to hold across `.await`
/// implement it there.
pub trait Lock<T> {
    /// Guard giving exclusive access until dropped.
    type Guard<'a>: DerefMut<Target = T>
    where
        Self: 'a;

    /// Wraps `value`.
    fn new(value: T) -> Self;

    /// Acquires the lock.
    async fn lock(&self) -> Self::Guard<'_>;

    /// Acquires the lock if it is free, without blocking or awaiting.
    fn try_lock(&self) -> Option<Self::Guard<'_>>;

    /// Borrows the value without locking.
    fn get_mut(&mut self) -> &mut T;

    /// Returns the value.
    fn into_inner(self) -> T;
}

/// Selects a lock type for any `T`, so users write `Volume<F, Spin>` rather
/// than `Volume<F, spin::Mutex<F>>`.
pub trait LockKind {
    /// The lock wrapping a `T`.
    type Lock<T>: Lock<T>;
}

}

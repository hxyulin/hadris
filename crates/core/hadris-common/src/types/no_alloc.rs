use core::fmt;
use core::ops::{Index, IndexMut};

/// A fixed-capacity vector.
#[derive(Debug)]
pub struct ArrayVec<T, const N: usize> {
    inner: heapless::Vec<T, N>,
}

/// An error returned when an [`ArrayVec`] has no remaining capacity.
#[derive(Debug, Clone)]
pub enum ArrayVecError {
    /// The vector is full.
    CapacityOverflow,
}

impl fmt::Display for ArrayVecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityOverflow => f.write_str("capacity overflow"),
        }
    }
}

impl core::error::Error for ArrayVecError {}

impl<T, const N: usize> ArrayVec<T, N> {
    /// Creates an empty vector.
    pub const fn new() -> Self {
        Self {
            inner: heapless::Vec::new(),
        }
    }

    /// Appends a value, returning an error when the vector is full.
    pub fn try_push(&mut self, value: T) -> Result<(), ArrayVecError> {
        self.inner
            .push(value)
            .map_err(|_| ArrayVecError::CapacityOverflow)
    }

    /// Appends a value.
    ///
    /// # Panics
    ///
    /// Panics when the vector is full.
    pub fn push(&mut self, value: T) {
        self.try_push(value).expect("ArrayVec: ran out of capacity");
    }

    /// Returns the number of stored values.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the stored values as a slice.
    pub fn as_slice(&self) -> &[T] {
        self.inner.as_slice()
    }

    /// Returns the stored values as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.inner.as_mut_slice()
    }

    /// Returns an iterator over the stored values.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.inner.iter()
    }

    /// Returns a mutable iterator over the stored values.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.inner.iter_mut()
    }

    /// Reverses the stored values.
    pub fn reverse(&mut self) {
        self.inner.reverse();
    }

    /// Returns a pointer to the vector's storage.
    pub fn as_ptr(&self) -> *const T {
        self.inner.as_ptr()
    }

    /// Returns a mutable pointer to the vector's storage.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.inner.as_mut_ptr()
    }
}

impl<T, const N: usize> Default for ArrayVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> ArrayVec<T, N>
where
    T: Copy + PartialEq,
{
    /// Returns whether the vector contains the given value.
    pub fn contains(&self, value: &T) -> bool {
        self.inner.contains(value)
    }
}

impl<T, const N: usize> ArrayVec<T, N>
where
    T: Copy + Ord,
{
    /// Sorts the vector without preserving the order of equal values.
    pub fn sort_unstable(&mut self) {
        self.inner.sort_unstable();
    }
}

impl<T, const N: usize> Index<usize> for ArrayVec<T, N>
where
    T: Copy,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[index]
    }
}

impl<T, const N: usize> IndexMut<usize> for ArrayVec<T, N>
where
    T: Copy,
{
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.inner[index]
    }
}

/// A fixed-capacity FIFO ring buffer.
///
/// A buffer with storage size `N` can hold at most `N - 1` values.
#[derive(Clone, Copy)]
pub struct RingBuf<T: Copy, const N: usize> {
    buf: [Option<T>; N],
    head: usize,
    tail: usize,
}

impl<T: Copy, const N: usize> RingBuf<T, N> {
    /// The number of storage slots in the buffer.
    pub const SIZE: usize = N;

    /// Creates an empty ring buffer.
    pub const fn new() -> Self {
        Self {
            buf: [None; N],
            head: 0,
            tail: 0,
        }
    }

    /// Returns whether the buffer is empty.
    pub const fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Returns the number of stored values.
    pub const fn len(&self) -> usize {
        (self.head + Self::SIZE - self.tail) % Self::SIZE
    }

    /// Returns whether the buffer is full.
    pub const fn is_full(&self) -> bool {
        (self.head + 1) % N == self.tail
    }

    /// Returns the maximum number of values the buffer can hold.
    pub const fn max_capacity(&self) -> usize {
        Self::SIZE - 1
    }

    /// Appends a value, returning it when the buffer is full.
    pub fn try_push(&mut self, value: T) -> Result<(), T> {
        if self.is_full() {
            return Err(value);
        }

        // SAFETY: The buffer was checked for remaining capacity.
        unsafe { self.push_unchecked(value) };
        Ok(())
    }

    /// Appends a value.
    ///
    /// # Panics
    ///
    /// Panics when the buffer is full.
    pub fn push(&mut self, value: T) {
        if self.try_push(value).is_err() {
            panic!("ringbuf is full");
        }
    }

    /// Appends a value without checking whether the buffer is full.
    ///
    /// # Safety
    ///
    /// The caller must ensure the buffer is not full.
    pub unsafe fn push_unchecked(&mut self, value: T) {
        self.buf[self.head] = Some(value);
        self.head = (self.head + 1) % N;
    }

    /// Removes and returns the oldest value.
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        let value = self.buf[self.tail].take();
        self.tail = (self.tail + 1) % N;
        value
    }
}

impl<T: Copy, const N: usize> Default for RingBuf<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    static_assertions::assert_impl_all!(RingBuf<u8, 4>: Clone, Copy);

    #[test]
    fn array_vec_preserves_values_and_capacity_errors() {
        let mut values = ArrayVec::<u8, 2>::new();
        values.push(2);
        values.push(1);
        assert!(matches!(
            values.try_push(3),
            Err(ArrayVecError::CapacityOverflow)
        ));
        values.sort_unstable();
        assert_eq!(values.as_slice(), &[1, 2]);
    }

    #[test]
    fn array_vec_drops_stored_values() {
        static DROPS: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;

        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROPS.store(0, Ordering::Relaxed);
        {
            let mut values = ArrayVec::<DropCounter, 2>::new();
            values.push(DropCounter);
            values.push(DropCounter);
        }
        assert_eq!(DROPS.load(Ordering::Relaxed), 2);
    }

    #[test]
    #[should_panic]
    fn array_vec_rejects_uninitialized_index() {
        let mut values = ArrayVec::<u8, 2>::new();
        values.push(1);
        let _ = values[1];
    }

    #[test]
    fn ring_buffer_wraps_and_preserves_fifo_order() {
        let mut values = RingBuf::<u8, 4>::new();
        values.push(1);
        values.push(2);
        values.push(3);
        assert!(values.is_full());
        assert_eq!(values.try_push(4), Err(4));
        assert_eq!(values.pop(), Some(1));
        values.push(4);
        assert_eq!(values.pop(), Some(2));
        assert_eq!(values.pop(), Some(3));
        assert_eq!(values.pop(), Some(4));
        assert_eq!(values.pop(), None);
    }
}

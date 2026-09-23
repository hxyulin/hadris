use core::iter::FusedIterator;

/// An iterator over `Result`s that ends after its first `Err`.
///
/// Every Hadris iterator yielding `Result` yields at most one `Err` and then
/// `None`, so a `for` loop over a corrupt directory cannot spin on the same
/// error. Format crates wrap their raw iterators in this adapter.
///
/// ```rust
/// use hadris_fs::FuseOnError;
///
/// let items = [Ok(1), Err("bad"), Ok(2)];
/// let seen: Vec<_> = FuseOnError::new(items.into_iter()).collect();
/// assert_eq!(seen, [Ok(1), Err("bad")]);
/// ```
#[derive(Debug, Clone)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct FuseOnError<I> {
    inner: Option<I>,
}

impl<I> FuseOnError<I> {
    /// Wraps `inner`.
    pub const fn new(inner: I) -> Self {
        Self { inner: Some(inner) }
    }

    /// Whether the iterator has ended, after an `Err` or a `None`.
    pub const fn is_done(&self) -> bool {
        self.inner.is_none()
    }

    /// The wrapped iterator, or `None` once it has ended.
    pub fn into_inner(self) -> Option<I> {
        self.inner
    }
}

impl<I, T, E> Iterator for FuseOnError<I>
where
    I: Iterator<Item = Result<T, E>>,
{
    type Item = Result<T, E>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.inner.as_mut()?.next();
        if !matches!(item, Some(Ok(_))) {
            self.inner = None;
        }
        item
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.inner {
            Some(inner) => (0, inner.size_hint().1),
            None => (0, Some(0)),
        }
    }
}

impl<I, T, E> FusedIterator for FuseOnError<I> where I: Iterator<Item = Result<T, E>> {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Repeat(u8);

    impl Iterator for Repeat {
        type Item = Result<u8, &'static str>;

        fn next(&mut self) -> Option<Self::Item> {
            self.0 += 1;
            Some(if self.0 % 2 == 0 {
                Err("corrupt")
            } else {
                Ok(self.0)
            })
        }
    }

    #[test]
    fn stops_after_the_first_error() {
        let mut it = FuseOnError::new(Repeat(0));
        assert_eq!(it.next(), Some(Ok(1)));
        assert_eq!(it.next(), Some(Err("corrupt")));
        assert!(it.is_done());
        assert_eq!(it.next(), None);
        assert_eq!(it.next(), None);
        assert_eq!(it.size_hint(), (0, Some(0)));
    }

    #[test]
    fn stays_ended_after_none() {
        struct Resumes(u8);
        impl Iterator for Resumes {
            type Item = Result<u8, ()>;
            fn next(&mut self) -> Option<Self::Item> {
                self.0 += 1;
                (self.0 != 2).then_some(Ok(self.0))
            }
        }
        let items: [Option<Result<u8, ()>>; 3] = {
            let mut it = FuseOnError::new(Resumes(0));
            [it.next(), it.next(), it.next()]
        };
        assert_eq!(items, [Some(Ok(1)), None, None]);
    }
}

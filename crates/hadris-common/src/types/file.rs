use core::{fmt, ops::Range};

/// A fixed-length, stack-allocated filename buffer.
///
/// Stores up to `N` bytes of filename data on the stack without
/// heap allocation. Useful for no-std filesystem implementations
/// where filenames have a known maximum length.
///
/// # Example
///
/// ```rust
/// use hadris_common::types::file::FixedFilename;
///
/// let name = FixedFilename::<64>::from(b"readme.txt".as_slice());
/// assert_eq!(name.len(), 10);
/// assert_eq!(name.as_str(), "readme.txt");
/// assert!(!name.is_empty());
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FixedFilename<const N: usize> {
    pub data: [u8; N],
    pub len: usize,
}

impl<const N: usize> FixedFilename<N> {
    pub const fn empty() -> Self {
        Self {
            data: [0; N],
            len: 0,
        }
    }

    pub const fn with_size(size: usize) -> Self {
        assert!(size <= N);
        Self {
            data: [0; N],
            len: size,
        }
    }

    /// Borrow the contents as a `&str`.
    ///
    /// # Panics
    ///
    /// Panics if the bytes are not valid UTF-8. Use [`Self::try_as_str`] for
    /// a fallible variant. Note: prior versions used `from_utf8_unchecked`,
    /// which was unsound because [`Self::as_bytes_mut`] (and the other
    /// byte-level constructors) accept arbitrary bytes safely.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).expect("FixedFilename contains invalid UTF-8")
    }

    /// Borrow the contents as a `&str` if they are valid UTF-8.
    pub fn try_as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.as_bytes())
    }

    pub fn allocate(&mut self, bytes: usize) {
        let len = self.len;
        assert!(bytes + len <= N);
        self.len += bytes;
        //self.data[len..self.len]
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[0..self.len]
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data[0..self.len]
    }

    pub fn truncate(&mut self, new_size: usize) {
        assert!(new_size <= N);
        self.len = new_size;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push_slice(&mut self, slice: &[u8]) -> Range<usize> {
        assert!(self.len + slice.len() <= self.data.len());
        let start = self.len;
        self.len += slice.len();
        self.data[start..self.len].copy_from_slice(slice);
        start..self.len
    }

    pub fn push_byte(&mut self, b: u8) -> usize {
        assert!(self.len < N);
        self.data[self.len] = b;
        self.len += 1;
        self.len - 1
    }

    pub fn try_push_slice(&mut self, slice: &[u8]) -> Option<Range<usize>> {
        if self.len + slice.len() > N {
            return None;
        }
        Some(self.push_slice(slice))
    }

    pub fn try_push_byte(&mut self, b: u8) -> Option<usize> {
        if self.len >= N {
            return None;
        }
        Some(self.push_byte(b))
    }

    pub fn remaining_capacity(&self) -> usize {
        N - self.len
    }
}

impl<const N: usize> fmt::Debug for FixedFilename<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_as_str() {
            Ok(s) => f.debug_tuple("FixedFilename").field(&s).finish(),
            Err(_) => f
                .debug_tuple("FixedFilename")
                .field(&self.as_bytes())
                .finish(),
        }
    }
}

impl<const N: usize> fmt::Display for FixedFilename<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.try_as_str() {
            Ok(s) => f.write_str(s),
            Err(_) => write!(f, "{:?}", self.as_bytes()),
        }
    }
}

impl<const N: usize> From<&[u8]> for FixedFilename<N> {
    fn from(value: &[u8]) -> Self {
        assert!(value.len() <= N);
        let mut str = FixedFilename::with_size(value.len());
        str.data[0..value.len()].copy_from_slice(value);
        str
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_as_str_rejects_invalid_utf8() {
        // Regression test for issue #26: previously `as_str` used
        // `from_utf8_unchecked`, which made it possible to invoke UB through
        // entirely safe code by writing arbitrary bytes via `as_bytes_mut`.
        let mut filename = FixedFilename::<10>::with_size(10);
        filename.as_bytes_mut()[0] = 0xff;

        assert!(filename.try_as_str().is_err());
    }

    #[test]
    fn debug_does_not_panic_on_invalid_utf8() {
        use core::fmt::Write as _;
        struct Sink;
        impl core::fmt::Write for Sink {
            fn write_str(&mut self, _: &str) -> core::fmt::Result {
                Ok(())
            }
        }
        let mut filename = FixedFilename::<10>::with_size(10);
        filename.as_bytes_mut()[0] = 0xff;
        // Should fall back to byte-slice formatting instead of panicking.
        write!(Sink, "{:?}", filename).unwrap();
        write!(Sink, "{}", filename).unwrap();
    }

    #[test]
    fn try_as_str_round_trips_valid_utf8() {
        let name = FixedFilename::<64>::from(b"readme.txt".as_slice());
        assert_eq!(name.try_as_str().unwrap(), "readme.txt");
        assert_eq!(name.as_str(), "readme.txt");
    }

    #[test]
    #[should_panic(expected = "invalid UTF-8")]
    fn as_str_panics_on_invalid_utf8() {
        let mut filename = FixedFilename::<10>::with_size(10);
        filename.as_bytes_mut()[0] = 0xff;
        let _ = filename.as_str();
    }
}

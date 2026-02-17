use core::{fmt, ops::Range};

/// A Fixed-Length Filename
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

    pub fn as_str(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(self.as_bytes()) }
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
        f.debug_tuple("FixedFilename")
            .field(&self.as_str())
            .finish()
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

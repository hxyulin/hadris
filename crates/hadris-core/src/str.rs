use core::ops::{Index, IndexMut, Range};

/// A no-std compatible string type
///
/// This is a wrapper around a fixed size array of bytes
/// The string is not null terminated, and the length is stored in the struct
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FixedByteStr<const N: usize> {
    pub raw: [u8; N],
    pub len: usize,
}

impl<const N: usize> FixedByteStr<N> {
    pub fn new() -> Self {
        Self {
            raw: [0; N],
            len: 0,
        }
    }
    pub fn from_str(s: &str) -> Self {
        assert!(s.len() <= N, "String length exceeds maximum length");
        let mut str = Self {
            raw: [0; N],
            len: s.len(),
        };
        str.raw[..s.len()].copy_from_slice(s.as_bytes());
        str
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.raw[..self.len]).unwrap()
    }

    pub fn as_ascii_str(&self) -> &AsciiStr {
        AsciiStr::from_bytes(&self.raw[..self.len])
    }

    pub fn as_slice(&self) -> &[u8; N] {
        &self.raw
    }
}

impl<const N: usize> core::fmt::Debug for FixedByteStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("FixedByteStr").field(&self.as_str()).finish()
    }
}

impl<const N: usize> core::fmt::Display for FixedByteStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> core::fmt::Write for FixedByteStr<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let len = s.len();
        let remaining = N.saturating_sub(self.len);

        if len > remaining {
            return Err(core::fmt::Error);
        }

        self.raw[self.len..self.len + len].copy_from_slice(s.as_bytes());

        self.len += len;
        Ok(())
    }
}

#[cfg(feature = "alloc")]
pub struct AsciiString {
    raw: alloc::vec::Vec<u8>,
}

#[cfg(feature = "alloc")]
impl AsciiString {}

#[repr(transparent)]
#[derive(PartialEq, Eq, Hash)]
pub struct AsciiStr([u8]);

impl AsciiStr {
    pub fn from_str<T: AsAsciiStr + ?Sized>(s: &T) -> &Self {
        s.as_ascii_str()
    }

    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { core::mem::transmute(bytes) }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap()
    }

    pub fn as_chars(&self) -> &[u8] {
        &self.0
    }

    pub fn ends_with(&self, c: u8) -> bool {
        self.0.last().map(|l| l == &c).unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn substr(&self, range: Range<usize>) -> &Self {
        assert!(range.start <= range.end);
        assert!(range.end <= self.len());
        unsafe { core::mem::transmute(&self.0[range]) }
    }

    pub fn rfind(&self, c: u8) -> Option<usize> {
        self.0.iter().rposition(|b| *b == c)
    }

    pub fn find(&self, c: u8) -> Option<usize> {
        self.0.iter().position(|b| *b == c)
    }
}

impl<'a> From<&'a str> for &'a AsciiStr {
    fn from(s: &str) -> &AsciiStr {
        AsciiStr::from_bytes(s.as_bytes())
    }
}

impl core::fmt::Display for AsciiStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::fmt::Debug for AsciiStr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("AsciiStr").field(&self.as_str()).finish()
    }
}

impl Index<usize> for AsciiStr {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl Index<core::ops::Range<usize>> for AsciiStr {
    type Output = [u8];

    fn index(&self, index: core::ops::Range<usize>) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for AsciiStr {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

#[cfg(feature = "alloc")]
impl AsciiStr {
    pub fn to_string(&self) -> AsciiString {
        todo!()
    }
}

pub trait AsAsciiStr {
    fn as_ascii_str(&self) -> &AsciiStr;
}

impl AsAsciiStr for AsciiStr {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        self
    }
}

impl<'a> AsAsciiStr for &'a AsciiStr {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        self
    }
}

impl<'a, const N: usize> AsAsciiStr for &FixedByteStr<N> {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        AsciiStr::from_bytes(self.as_slice())
    }
}

impl<const N: usize> AsAsciiStr for FixedByteStr<N> {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        AsciiStr::from_bytes(self.as_slice())
    }
}

impl<'a> AsAsciiStr for &'a str {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        AsciiStr::from_bytes(self.as_bytes())
    }
}

impl AsAsciiStr for str {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        AsciiStr::from_bytes(self.as_bytes())
    }
}

#[cfg(feature = "alloc")]
mod _alloc_impls {
    use super::*;

    impl AsAsciiStr for alloc::string::String {
        fn as_ascii_str(&self) -> &AsciiStr {
            AsciiStr::from_str(self.as_str())
        }
    }

    impl AsAsciiStr for alloc::borrow::Cow<'_, str> {
        fn as_ascii_str(&self) -> &AsciiStr {
            AsciiStr::from_str(self.as_ref())
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn fixed_byte_test_str() {
        let mut str = FixedByteStr::<11>::new();
        str.write_str("Hello World").unwrap();
        assert_eq!(str.as_str(), "Hello World");
    }

    #[test]
    fn fixed_byte_str_from_str() {
        let str = FixedByteStr::<11>::from_str("Hello World");
        assert_eq!(str.as_str(), "Hello World");
    }

    #[test]
    fn fixed_byte_str_str_overflow() {
        let mut str = FixedByteStr::<11>::new();
        str.write_str("Hello World").unwrap();
        assert!(str.write_str("Hello World").is_err());
    }

    #[test]
    fn fixed_byte_str_str_display() {
        let str = FixedByteStr::<11>::from_str("Hello World");
        assert_eq!(format!("{}", str), "Hello World");
    }

    #[test]
    #[should_panic]
    fn fixed_byte_str_str_from_str_overflow() {
        _ = FixedByteStr::<11>::from_str("Hello World!");
    }

    #[test]
    fn test_ascii_str_from_bytes() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert_eq!(str.as_str(), "Hello World");
    }

    #[test]
    fn test_ascii_str_ends_with() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert!(str.ends_with(b'd'));
        assert!(!str.ends_with(b'!'));
    }

    #[test]
    fn test_ascii_str_is_empty() {
        let str = AsciiStr::from_bytes(b"");
        assert!(str.is_empty());

        let str = AsciiStr::from_bytes(b"Hello World");
        assert!(!str.is_empty());
    }

    #[test]
    fn test_ascii_str_len() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert_eq!(str.len(), 11);
    }

    #[test]
    fn test_ascii_str_substr() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert_eq!(str.substr(0..5), AsciiStr::from_bytes(b"Hello"));
        assert_eq!(str.substr(5..11), AsciiStr::from_bytes(b" World"));
    }

    #[test]
    fn test_ascii_str_rfind() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert_eq!(str.rfind(b'd'), Some(10));
        assert_eq!(str.rfind(b'l'), Some(9));
        assert_eq!(str.rfind(b'!'), None);
    }

    #[test]
    fn test_ascii_str_find() {
        let str = AsciiStr::from_bytes(b"Hello World");
        assert_eq!(str.find(b'd'), Some(10));
        assert_eq!(str.find(b'l'), Some(2));
        assert_eq!(str.find(b'!'), None);
    }
}

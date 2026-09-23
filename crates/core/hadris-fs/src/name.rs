use core::fmt;
use core::str::Utf8Error;

use crate::ErrorKind;

/// Why a byte string is not a valid [`Name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name is `.`.
    CurrentDir,
    /// The name is `..`.
    ParentDir,
    /// The name contains `/`.
    Separator,
    /// The name contains a NUL byte.
    Nul,
    /// The name does not fit the destination buffer.
    TooLong,
}

impl NameError {
    /// Returns the matching error kind.
    pub const fn kind(self) -> ErrorKind {
        match self {
            Self::TooLong => ErrorKind::LimitExceeded,
            _ => ErrorKind::InvalidInput,
        }
    }
}

impl From<NameError> for ErrorKind {
    fn from(err: NameError) -> Self {
        err.kind()
    }
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "name is empty",
            Self::CurrentDir => "name is `.`",
            Self::ParentDir => "name is `..`",
            Self::Separator => "name contains `/`",
            Self::Nul => "name contains a NUL byte",
            Self::TooLong => "name is too long for the buffer",
        })
    }
}

impl core::error::Error for NameError {}

const fn validate(bytes: &[u8]) -> Result<(), NameError> {
    match bytes {
        [] => return Err(NameError::Empty),
        [b'.'] => return Err(NameError::CurrentDir),
        [b'.', b'.'] => return Err(NameError::ParentDir),
        _ => {}
    }
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' => return Err(NameError::Separator),
            0 => return Err(NameError::Nul),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// A single directory-entry name, as bytes.
///
/// Like [`str`] or `std::path::Path`, `Name` is unsized and used behind a
/// reference. A valid name is non-empty, is not `.` or `..`, and contains no
/// `/` or NUL byte. The encoding is filesystem-defined, so the bytes need not
/// be UTF-8.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Name([u8]);

impl Name {
    /// Validates `bytes` as a name.
    pub fn new<B: AsRef<[u8]> + ?Sized>(bytes: &B) -> Result<&Name, NameError> {
        Self::from_bytes(bytes.as_ref())
    }

    /// Validates `bytes` as a name in a `const` context.
    pub const fn from_bytes(bytes: &[u8]) -> Result<&Name, NameError> {
        match validate(bytes) {
            Ok(()) => Ok(Self::from_bytes_unchecked(bytes)),
            Err(err) => Err(err),
        }
    }

    const fn from_bytes_unchecked(bytes: &[u8]) -> &Name {
        // SAFETY: `Name` is `repr(transparent)` over `[u8]`, so the pointer
        // cast preserves layout, metadata and lifetime.
        unsafe { &*(bytes as *const [u8] as *const Name) }
    }

    /// Returns the name's bytes.
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the name as UTF-8 text.
    pub const fn to_str(&self) -> Result<&str, Utf8Error> {
        core::str::from_utf8(&self.0)
    }

    /// Returns the length in bytes. Always non-zero.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for chunk in self.0.utf8_chunks() {
            for c in chunk.valid().chars() {
                write!(f, "{}", c.escape_debug())?;
            }
            for byte in chunk.invalid() {
                write!(f, "\\x{byte:02x}")?;
            }
        }
        f.write_str("\"")
    }
}

impl AsRef<[u8]> for Name {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<Name> for Name {
    fn as_ref(&self) -> &Name {
        self
    }
}

impl<'a> TryFrom<&'a str> for &'a Name {
    type Error = NameError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        Name::new(value)
    }
}

impl<'a> TryFrom<&'a [u8]> for &'a Name {
    type Error = NameError;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        Name::new(value)
    }
}

/// A fixed-capacity name buffer that works without an allocator.
///
/// The buffer is either empty or holds a valid [`Name`] of at most `N` bytes.
/// Filesystems write directory entry names into it with
/// [`set`](Self::set) or, to avoid a second copy, [`fill`](Self::fill);
/// callers read them back with [`as_name`](Self::as_name).
#[derive(Clone)]
pub struct NameBuf<const N: usize = 255> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> NameBuf<N> {
    /// The buffer's capacity in bytes.
    pub const CAPACITY: usize = N;

    /// Creates an empty buffer.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// Creates a buffer holding `name`.
    pub fn from_name(name: &Name) -> Result<Self, NameError> {
        let mut buf = Self::new();
        buf.set(name)?;
        Ok(buf)
    }

    /// Replaces the contents with `name`. On error the buffer is unchanged.
    pub fn set(&mut self, name: &Name) -> Result<(), NameError> {
        let bytes = name.as_bytes();
        let dst = self.buf.get_mut(..bytes.len()).ok_or(NameError::TooLong)?;
        dst.copy_from_slice(bytes);
        self.len = bytes.len();
        Ok(())
    }

    /// Validates `bytes` and replaces the contents. On error the buffer is unchanged.
    pub fn set_bytes(&mut self, bytes: &[u8]) -> Result<(), NameError> {
        self.set(Name::new(bytes)?)
    }

    /// Lets `write` fill the buffer in place, then validates the result.
    ///
    /// `write` gets the whole `N`-byte buffer and returns how many bytes it
    /// wrote, so a driver can decode or join name pieces straight into it. On
    /// any error the buffer is left empty.
    ///
    /// ```rust
    /// use hadris_fs::{NameBuf, NameError};
    ///
    /// let mut name = NameBuf::<16>::new();
    /// name.fill(|buf| {
    ///     let pieces: [&[u8]; 2] = [b"long", b"name.txt"];
    ///     let mut len = 0;
    ///     for piece in pieces {
    ///         let end = len + piece.len();
    ///         buf.get_mut(len..end).ok_or(NameError::TooLong)?.copy_from_slice(piece);
    ///         len = end;
    ///     }
    ///     Ok(len)
    /// })?;
    /// assert_eq!(name.as_bytes(), b"longname.txt");
    /// # Ok::<(), NameError>(())
    /// ```
    pub fn fill(
        &mut self,
        write: impl FnOnce(&mut [u8]) -> Result<usize, NameError>,
    ) -> Result<(), NameError> {
        self.len = 0;
        let len = write(&mut self.buf)?;
        let bytes = self.buf.get(..len).ok_or(NameError::TooLong)?;
        validate(bytes)?;
        self.len = len;
        Ok(())
    }

    /// Empties the buffer.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Returns the stored name, or `None` if the buffer is empty.
    pub fn as_name(&self) -> Option<&Name> {
        if self.len == 0 {
            None
        } else {
            Some(Name::from_bytes_unchecked(&self.buf[..self.len]))
        }
    }

    /// Returns the stored bytes, empty if no name is stored.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Returns the stored length in bytes.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether no name is stored.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the buffer's capacity in bytes.
    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<const N: usize> Default for NameBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> PartialEq for NameBuf<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl<const N: usize> Eq for NameBuf<N> {}

impl<const N: usize> core::hash::Hash for NameBuf<N> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl<const N: usize> fmt::Debug for NameBuf<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_name() {
            Some(name) => fmt::Debug::fmt(name, f),
            None => f.write_str("\"\""),
        }
    }
}

#[cfg(feature = "alloc")]
mod owned {
    use alloc::borrow::ToOwned;
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::borrow::Borrow;
    use core::fmt;
    use core::ops::Deref;

    use super::{Name, NameError, validate};

    /// An owned, heap-allocated [`Name`].
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct OwnedName(Box<[u8]>);

    impl OwnedName {
        /// Validates `bytes` and takes ownership of them.
        pub fn new(bytes: Vec<u8>) -> Result<Self, NameError> {
            validate(&bytes)?;
            Ok(Self(bytes.into_boxed_slice()))
        }

        /// Returns the borrowed name.
        pub fn as_name(&self) -> &Name {
            Name::from_bytes_unchecked(&self.0)
        }

        /// Returns the name's bytes.
        pub fn into_bytes(self) -> Vec<u8> {
            self.0.into_vec()
        }
    }

    impl Deref for OwnedName {
        type Target = Name;

        fn deref(&self) -> &Name {
            self.as_name()
        }
    }

    impl Borrow<Name> for OwnedName {
        fn borrow(&self) -> &Name {
            self.as_name()
        }
    }

    impl AsRef<Name> for OwnedName {
        fn as_ref(&self) -> &Name {
            self.as_name()
        }
    }

    impl ToOwned for Name {
        type Owned = OwnedName;

        fn to_owned(&self) -> OwnedName {
            OwnedName(Box::from(self.as_bytes()))
        }
    }

    impl From<&Name> for OwnedName {
        fn from(name: &Name) -> Self {
            name.to_owned()
        }
    }

    impl fmt::Debug for OwnedName {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self.as_name(), f)
        }
    }
}

#[cfg(feature = "alloc")]
pub use owned::OwnedName;

#[cfg(test)]
mod tests {
    #[test]
    fn fill_validates_in_place() {
        let mut buf = NameBuf::<8>::new();
        buf.fill(|b| {
            b[..3].copy_from_slice(b"abc");
            Ok(3)
        })
        .unwrap();
        assert_eq!(buf.as_bytes(), b"abc");
        assert_eq!(
            buf.fill(|b| {
                b[..2].copy_from_slice(b"..");
                Ok(2)
            }),
            Err(NameError::ParentDir)
        );
        assert!(buf.is_empty());
        assert_eq!(buf.fill(|_| Ok(9)), Err(NameError::TooLong));
        assert_eq!(buf.fill(|_| Err(NameError::Nul)), Err(NameError::Nul));
        assert!(buf.as_name().is_none());
    }

    use super::*;
    use alloc::format;

    #[test]
    fn name_accepts_ordinary_bytes() {
        let name = Name::new("readme.md").unwrap();
        assert_eq!(name.as_bytes(), b"readme.md");
        assert_eq!(name.to_str(), Ok("readme.md"));
        assert_eq!(name.len(), 9);
        assert!(Name::new("...").is_ok());
        assert!(Name::new(".hidden").is_ok());
        assert!(Name::new("a\\b").is_ok());
    }

    #[test]
    fn name_rejects_invalid_input() {
        assert_eq!(Name::new(""), Err(NameError::Empty));
        assert_eq!(Name::new("."), Err(NameError::CurrentDir));
        assert_eq!(Name::new(".."), Err(NameError::ParentDir));
        assert_eq!(Name::new("a/b"), Err(NameError::Separator));
        assert_eq!(Name::new("/"), Err(NameError::Separator));
        assert_eq!(Name::new(b"a\0b"), Err(NameError::Nul));
        assert_eq!(NameError::Nul.kind(), ErrorKind::InvalidInput);
        assert_eq!(NameError::TooLong.kind(), ErrorKind::LimitExceeded);
    }

    #[test]
    fn name_allows_non_utf8() {
        let name = Name::new(&[0xff, b'a']).unwrap();
        assert!(name.to_str().is_err());
        assert_eq!(format!("{name:?}"), "\"\\xffa\"");
    }

    #[test]
    fn name_const_constructor() {
        const NAME: Result<&Name, NameError> = Name::from_bytes(b"boot");
        assert_eq!(NAME.unwrap().as_bytes(), b"boot");
    }

    #[test]
    fn name_buf_round_trip() {
        let mut buf = NameBuf::<8>::new();
        assert!(buf.is_empty());
        assert_eq!(buf.as_name(), None);
        assert_eq!(buf.capacity(), 8);
        buf.set(Name::new("kernel").unwrap()).unwrap();
        assert_eq!(buf.as_name().unwrap().as_bytes(), b"kernel");
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.set_bytes(b"too-long-name"), Err(NameError::TooLong));
        assert_eq!(buf.as_bytes(), b"kernel");
        assert_eq!(buf.set_bytes(b".."), Err(NameError::ParentDir));
        buf.set_bytes(b"exactly8").unwrap();
        assert_eq!(buf.as_bytes(), b"exactly8");
        buf.clear();
        assert_eq!(buf, NameBuf::new());
    }

    #[test]
    fn name_buf_default_capacity() {
        let buf = NameBuf::<255>::from_name(Name::new(&[b'x'; 255]).unwrap()).unwrap();
        assert_eq!(buf.len(), 255);
        assert_eq!(NameBuf::<255>::CAPACITY, 255);
        let buf: NameBuf = NameBuf::default();
        assert_eq!(buf.capacity(), 255);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn owned_name() {
        use alloc::borrow::ToOwned;
        let owned = Name::new("a.txt").unwrap().to_owned();
        assert_eq!(owned.as_bytes(), b"a.txt");
        assert_eq!(OwnedName::new(b"a/b".to_vec()), Err(NameError::Separator));
        assert_eq!(owned.into_bytes(), b"a.txt");
    }
}

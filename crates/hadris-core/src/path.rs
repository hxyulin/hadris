//! This module contains structures and functions for working with paths.
//! Mainly, the [`Path`] struct is used to represent a path on the filesystem.
//! This is designed as a wrapper around an [`AsciiStr`], which is a string slice of ASCII characters.
//! UTF-8 is not yet supported.
//! TODO: Add support for UTF-8 paths

use crate::str::{AsAsciiStr, AsciiStr};

/// A path on the filesystem, which is a borrowed [`AsciiStr`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path<'a>(&'a AsciiStr);

impl<'a> AsAsciiStr for Path<'a> {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        self.0
    }
}

/// A wrapper around a Path, which represents the basename of the path
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathBase<'a>(&'a AsciiStr);

impl<'a> Path<'a> {
    /// Creates a new [`Path`] from a borrowed [`AsciiStr`], or any type that implements [`AsAsciiStr`].
    pub fn new<T: AsAsciiStr + ?Sized>(path: &'a T) -> Self {
        Self(path.as_ascii_str())
    }

    /// Returns true if the path has a trailing slash.
    ///
    /// The specification says that a path with a trailing slash is a directory
    pub fn has_trailing_slash(&self) -> bool {
        self.0.ends_with(b'/')
    }

    pub fn is_absolute(&self) -> bool {
        self.0.starts_with(b'/')
    }

    /// Returns true if the path is the root directory.
    pub fn is_root(&self) -> bool {
        self.0.len() == 1 && self.0[0] == b'/'
    }

    /// Returns the parent directory of the path.
    ///
    /// If the path is the root directory, returns `None`.
    pub fn get_parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let path = if self.has_trailing_slash() {
            self.0.substr(0..self.0.len() - 1)
        } else {
            self.0
        };

        match path.rfind(b'/') {
            Some(0) => Some(Path::new("/")),
            Some(index) => Some(Path::new(path.substr(0..index))),
            None => Some(Path::new("/")),
        }
    }

    /// Returns the basename of the path, which is the part of the path without the parent directory
    pub fn basename(&self) -> Option<PathBase> {
        if self.is_root() {
            return None;
        }
        let path = if self.has_trailing_slash() {
            self.0.substr(0..self.0.len() - 1)
        } else {
            self.0
        };
        let index = path.rfind(b'/').map(|index| index + 1).unwrap_or(0);
        Some(PathBase(path.substr(index..path.len())))
    }

    /// Returns the filename of the path, should be called in stem paths
    /// TODO: Rename or remove
    pub fn filename(&self) -> Self {
        let index = self.0.rfind(b'.').unwrap_or(self.0.len());
        Path::new(self.0.substr(0..index))
    }

    /// Returns the extension of the path.
    /// TODO: Rename or remove
    pub fn extension(&self) -> Option<Self> {
        let index = self.0.rfind(b'.')?;
        if index == self.0.len() - 1 {
            return None;
        }
        Some(Path::new(self.0.substr(index + 1..self.0.len())))
    }

    /// Returns the underlying [`AsciiStr`].
    pub fn as_ascii_str(&self) -> &AsciiStr {
        self.0
    }

    /// Returns the underlying string, converting the [`AsciiStr`] to a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl PathBase<'_> {
    /// Gets the stem of the path.
    ///
    /// The stem is the part of the path without the extension.
    pub fn stem(&self) -> Option<&AsciiStr> {
        let dot_index = self.0.rfind(b'.').unwrap_or(self.0.len());
        Some(self.0.substr(0..dot_index))
    }

    /// Gets the extension of the path.
    ///
    /// The extension is the part of the path after the last dot.
    pub fn extension(&self) -> Option<&AsciiStr> {
        let dot_index = self.0.rfind(b'.').unwrap_or(self.0.len());
        if dot_index == self.0.len() - 1 {
            return None;
        }
        Some(self.0.substr(dot_index + 1..self.0.len()))
    }

    /// Returns the underlying [`AsciiStr`].
    pub fn as_ascii_str(&self) -> &AsciiStr {
        self.0
    }

    /// Returns the underlying string, converting the [`AsciiStr`] to a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl core::fmt::Display for Path<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::fmt::Display for PathBase<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_path_has_trailing_slash() {
        let path = Path::new("/test/");
        assert!(path.has_trailing_slash());
        let path = Path::new("/test");
        assert!(!path.has_trailing_slash());
    }

    #[test]
    fn test_path_is_root() {
        let path = Path::new("/");
        assert!(path.is_root());
        let path = Path::new("/test");
        assert!(!path.is_root());
    }

    #[test]
    fn test_path_get_parent() {
        let path = Path::new("/");
        assert_eq!(path.get_parent(), None);
        let path = Path::new("/test");
        assert_eq!(path.get_parent(), Some(Path::new("/")));
        let path = Path::new("/test/test");
        assert_eq!(path.get_parent(), Some(Path::new("/test")));
        let path = Path::new("/test/boot/gluon.cfg");
        assert_eq!(path.get_parent(), Some(Path::new("/test/boot")));
    }

    #[test]
    fn test_path_basename() {
        let path = Path::new("/");
        assert_eq!(path.basename(), None);
        let path = Path::new("/test");
        assert_eq!(path.basename(), Some(PathBase("test".into())));
        let path = Path::new("/test/");
        assert_eq!(path.basename(), Some(PathBase("test".into())));
        let path = Path::new("/test/test");
        assert_eq!(path.basename(), Some(PathBase("test".into())));
        let path = Path::new("/test/test/");
        assert_eq!(path.basename(), Some(PathBase("test".into())));
        let path = Path::new("/test/boot/gluon.cfg");
        assert_eq!(path.basename(), Some(PathBase("gluon.cfg".into())));
    }
}

use crate::str::{AsAsciiStr, AsciiStr};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path<'a>(&'a AsciiStr);

impl<'a> AsAsciiStr for Path<'a> {
    #[inline]
    fn as_ascii_str(&self) -> &AsciiStr {
        self.0
    }
}

impl<'a> Path<'a> {
    pub fn new<T: AsAsciiStr + ?Sized>(path: &'a T) -> Self {
        Self(path.as_ascii_str())
    }

    pub fn has_trailing_slash(&self) -> bool {
        self.0.ends_with(b'/')
    }

    // TODO: This is not correct, but we dont have a 'current directory' concept yet
    pub fn is_root(&self) -> bool {
        self.0.is_empty() || self.0.len() == 1 && self.0[0] == b'/'
    }

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

    pub fn get_stem(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let path = if self.has_trailing_slash() {
            self.0.substr(0..self.0.len() - 1)
        } else {
            self.0
        };
        let index = path.rfind(b'/').map(|index| index + 1).unwrap_or(0);
        Some(Path::new(path.substr(index..path.len())))
    }

    /// Returns the filename of the path, should be called in stem paths
    pub fn filename(&self) -> Self {
        let index = self.0.rfind(b'.').unwrap_or(self.0.len());
        Path::new(self.0.substr(0..index))
    }

    pub fn extension(&self) -> Option<Self> {
        let index = self.0.rfind(b'.')?;
        if index == self.0.len() - 1 {
            return None;
        }
        Some(Path::new(self.0.substr(index + 1..self.0.len())))
    }

    pub fn as_ascii_str(&self) -> &AsciiStr {
        self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl core::fmt::Display for Path<'_> {
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
    fn test_path_get_stem() {
        let path = Path::new("/");
        assert_eq!(path.get_stem(), None);
        let path = Path::new("/test");
        assert_eq!(path.get_stem(), Some(Path::new("test")));
        let path = Path::new("/test/");
        assert_eq!(path.get_stem(), Some(Path::new("test")));
        let path = Path::new("/test/test");
        assert_eq!(path.get_stem(), Some(Path::new("test")));
        let path = Path::new("/test/test/");
        assert_eq!(path.get_stem(), Some(Path::new("test")));
        let path = Path::new("/test/boot/gluon.cfg");
        assert_eq!(path.get_stem(), Some(Path::new("gluon.cfg")));
    }
}

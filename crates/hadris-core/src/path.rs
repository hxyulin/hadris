use crate::str::AsciiStr;

pub struct Path<'a>(&'a AsciiStr);

impl<'a> Path<'a> {
    pub fn new(path: &'a AsciiStr) -> Self {
        Self(path)
    }

    pub fn has_trailing_slash(&self) -> bool {
        self.0.ends_with(b'/')
    }

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

        let index = match path.rfind(b'/') {
            Some(index) => index,
            None => return Some(Path::new("/".into()))
        };
        Some(Path::new(path.substr(0..index)))
    }

    pub fn get_stem(&self) -> Option<Self> {
        if self.is_root() {
            return Some(Path::new(self.0));
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

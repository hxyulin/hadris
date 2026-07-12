//! Lexical path handling for virtual filesystems and archives.
//!
//! Unlike `std::path`, this crate does not model host operating-system paths
//! and never performs filesystem I/O. Its borrowed path views and component
//! iterators are allocation-free and available in `no_std` environments.

#![no_std]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

/// Separator policy used while parsing a virtual path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Separators {
    /// Only `/` separates components.
    #[default]
    Slash,
    /// Both `/` and `\` separate components.
    SlashOrBackslash,
}

impl Separators {
    const fn matches(self, byte: u8) -> bool {
        byte == b'/' || matches!(self, Self::SlashOrBackslash) && byte == b'\\'
    }
}

/// A lexical component of a virtual path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component<'a> {
    /// One or more separators at the beginning of the path.
    Root,
    /// A `.` component.
    Current,
    /// A `..` component.
    Parent,
    /// A normal path component.
    Normal(&'a str),
}

/// A borrowed virtual path with an explicit separator policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VPath<'a> {
    raw: &'a str,
    separators: Separators,
}

impl<'a> VPath<'a> {
    /// Creates a slash-delimited virtual path.
    pub const fn new(path: &'a str) -> Self {
        Self::with_separators(path, Separators::Slash)
    }

    /// Creates a virtual path with the given separator policy.
    pub const fn with_separators(path: &'a str, separators: Separators) -> Self {
        Self {
            raw: path,
            separators,
        }
    }

    /// Returns the original, unnormalized path string.
    pub const fn as_str(self) -> &'a str {
        self.raw
    }

    /// Returns the separator policy.
    pub const fn separators(self) -> Separators {
        self.separators
    }

    /// Iterates over lexical components.
    pub fn components(self) -> Components<'a> {
        Components {
            path: self.raw,
            separators: self.separators,
            offset: 0,
            root_pending: self
                .raw
                .as_bytes()
                .first()
                .is_some_and(|byte| self.separators.matches(*byte)),
        }
    }

    /// Returns whether the path begins with a recognized separator.
    pub fn is_absolute(self) -> bool {
        matches!(self.components().next(), Some(Component::Root))
    }

    /// Returns the last normal component.
    pub fn file_name(self) -> Option<&'a str> {
        let mut result = None;
        for component in self.components() {
            match component {
                Component::Normal(name) => result = Some(name),
                Component::Current | Component::Root => {}
                Component::Parent => result = None,
            }
        }
        result
    }

    /// Splits the path into its raw parent view and final normal component.
    pub fn split_file(self) -> Option<(Self, &'a str)> {
        let bytes = self.raw.as_bytes();
        let mut end = bytes.len();
        while end > 0 && self.separators.matches(bytes[end - 1]) {
            end -= 1;
        }
        if end == 0 {
            return None;
        }
        let mut start = end;
        while start > 0 && !self.separators.matches(bytes[start - 1]) {
            start -= 1;
        }
        let name = &self.raw[start..end];
        if matches!(name, "." | "..") {
            return None;
        }
        let mut parent_end = start;
        while parent_end > 0 && self.separators.matches(bytes[parent_end - 1]) {
            parent_end -= 1;
        }
        if parent_end == 0 && start > 0 {
            parent_end = 1;
        }
        Some((
            Self::with_separators(&self.raw[..parent_end], self.separators),
            name,
        ))
    }

    /// Returns the raw parent path when this path ends in a normal component.
    pub fn parent(self) -> Option<Self> {
        self.split_file().map(|(parent, _)| parent)
    }
}

impl<'a> From<&'a str> for VPath<'a> {
    fn from(path: &'a str) -> Self {
        Self::new(path)
    }
}

impl AsRef<str> for VPath<'_> {
    fn as_ref(&self) -> &str {
        self.raw
    }
}

impl core::fmt::Display for VPath<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.raw)
    }
}

/// Allocation-free iterator over lexical components.
#[derive(Debug, Clone)]
pub struct Components<'a> {
    path: &'a str,
    separators: Separators,
    offset: usize,
    root_pending: bool,
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.path.as_bytes();
        if self.root_pending {
            self.root_pending = false;
            while self.offset < bytes.len() && self.separators.matches(bytes[self.offset]) {
                self.offset += 1;
            }
            return Some(Component::Root);
        }
        while self.offset < bytes.len() && self.separators.matches(bytes[self.offset]) {
            self.offset += 1;
        }
        if self.offset == bytes.len() {
            return None;
        }
        let start = self.offset;
        while self.offset < bytes.len() && !self.separators.matches(bytes[self.offset]) {
            self.offset += 1;
        }
        Some(match &self.path[start..self.offset] {
            "." => Component::Current,
            ".." => Component::Parent,
            normal => Component::Normal(normal),
        })
    }
}

/// An invalid lexical path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// A parent component would escape the virtual root.
    EscapesRoot,
}

impl core::fmt::Display for PathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EscapesRoot => f.write_str("parent component escapes the virtual root"),
        }
    }
}

impl core::error::Error for PathError {}

#[cfg(feature = "alloc")]
impl VPath<'_> {
    /// Normalizes separators and lexical `.`/`..` components.
    pub fn normalize(self) -> Result<alloc::string::String, PathError> {
        use alloc::string::String;
        use alloc::vec::Vec;

        let absolute = self.is_absolute();
        let mut stack = Vec::new();
        for component in self.components() {
            match component {
                Component::Root | Component::Current => {}
                Component::Normal(value) => stack.push(value),
                Component::Parent => {
                    stack.pop().ok_or(PathError::EscapesRoot)?;
                }
            }
        }
        let mut normalized = String::new();
        if absolute {
            normalized.push('/');
        }
        normalized.push_str(&stack.join("/"));
        Ok(normalized)
    }
}

/// Compatibility helper that returns a normalized `(directory, filename)` pair.
#[cfg(feature = "alloc")]
pub fn split_path(path: &str) -> Option<(alloc::string::String, alloc::string::String)> {
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    let mut parts = Vec::new();
    for component in VPath::new(path).components() {
        match component {
            Component::Normal(value) => parts.push(value),
            Component::Root | Component::Current => {}
            Component::Parent => return None,
        }
    }
    let filename = parts.last()?.to_string();
    let directory = if parts.len() > 1 {
        parts[..parts.len() - 1].join("/")
    } else {
        String::new()
    };
    Some((directory, filename))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_preserve_lexical_meaning() {
        let components: alloc::vec::Vec<_> = VPath::new("/a//./b/../c").components().collect();
        assert_eq!(
            components,
            [
                Component::Root,
                Component::Normal("a"),
                Component::Current,
                Component::Normal("b"),
                Component::Parent,
                Component::Normal("c"),
            ]
        );
    }

    #[test]
    fn separator_policy_is_explicit() {
        let slash: alloc::vec::Vec<_> = VPath::new(r"a\b/c").components().collect();
        assert_eq!(slash, [Component::Normal(r"a\b"), Component::Normal("c")]);
        let both: alloc::vec::Vec<_> =
            VPath::with_separators(r"a\b/c", Separators::SlashOrBackslash)
                .components()
                .collect();
        assert_eq!(
            both,
            [
                Component::Normal("a"),
                Component::Normal("b"),
                Component::Normal("c")
            ]
        );
    }

    #[test]
    fn parent_and_file_name_are_borrowed() {
        let path = VPath::new("/docs/api/readme.md/");
        assert_eq!(path.file_name(), Some("readme.md"));
        let (parent, name) = path.split_file().unwrap();
        assert_eq!(parent.as_str(), "/docs/api");
        assert_eq!(name, "readme.md");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn normalization_rejects_root_escape() {
        assert_eq!(VPath::new("a/./b/../c").normalize().unwrap(), "a/c");
        assert_eq!(VPath::new("../a").normalize(), Err(PathError::EscapesRoot));
        assert!(split_path("a/../file").is_none());
    }
}

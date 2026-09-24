//! Lexical path handling for virtual filesystems and archives.
//!
//! Unlike `std::path`, this module does not model host operating-system paths
//! and never performs filesystem I/O. Its borrowed path views and component
//! iterators are allocation-free and available in `no_std` environments.
//! Path parsing is a convenience; filesystem operations take [`Name`](crate::Name)s.

/// Separator policy used while parsing a virtual path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
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
///
/// ```
/// use hadris_fs::path::{Component, Separators, VPath};
///
/// let path = VPath::with_separators(r"boot\grub/../kernel.efi", Separators::SlashOrBackslash);
/// let components: Vec<_> = path.components().collect();
/// assert_eq!(
///     components,
///     [
///         Component::Normal("boot"),
///         Component::Normal("grub"),
///         Component::Parent,
///         Component::Normal("kernel.efi"),
///     ]
/// );
/// ```
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
#[non_exhaustive]
pub enum NormalizeError {
    /// A parent component would escape the virtual root.
    EscapesRoot,
}

impl NormalizeError {
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::EscapesRoot => "parent component escapes the virtual root",
        }
    }

    /// Returns the matching error kind.
    pub const fn kind(self) -> crate::ErrorKind {
        crate::ErrorKind::InvalidInput
    }
}

impl From<NormalizeError> for crate::ErrorKind {
    fn from(err: NormalizeError) -> Self {
        err.kind()
    }
}

impl core::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.description())
    }
}

impl core::error::Error for NormalizeError {}

#[cfg(feature = "alloc")]
impl VPath<'_> {
    /// Normalizes separators and lexical `.`/`..` components.
    pub fn normalize(self) -> Result<alloc::string::String, NormalizeError> {
        use alloc::string::String;
        use alloc::vec::Vec;

        let absolute = self.is_absolute();
        let mut stack = Vec::new();
        for component in self.components() {
            match component {
                Component::Root | Component::Current => {}
                Component::Normal(value) => stack.push(value),
                Component::Parent => {
                    stack.pop().ok_or(NormalizeError::EscapesRoot)?;
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
        assert_eq!(
            VPath::new("../a").normalize(),
            Err(NormalizeError::EscapesRoot)
        );
        assert!(split_path("a/../file").is_none());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn split_path_separates_the_last_component() {
        assert_eq!(
            split_path("file.txt").unwrap(),
            ("".into(), "file.txt".into())
        );
        assert_eq!(
            split_path("a/b/c/file.txt").unwrap(),
            ("a/b/c".into(), "file.txt".into())
        );
        assert_eq!(
            split_path("/root/file.txt").unwrap(),
            ("root".into(), "file.txt".into())
        );
        assert!(split_path("").is_none());
        assert!(split_path("/").is_none());
    }
}

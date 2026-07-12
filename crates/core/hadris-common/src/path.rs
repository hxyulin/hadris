//! Path utilities shared across filesystem implementations.

extern crate alloc;
use alloc::string::String;

pub use hadris_path::{Component, Components, PathError, Separators, VPath};

/// Splits a path into (directory, filename).
///
/// Returns `None` if the path is empty or contains only separators.
///
/// # Examples
/// ```
/// # use hadris_common::path::split_path;
/// assert_eq!(split_path("dir/file.txt"), Some(("dir".into(), "file.txt".into())));
/// assert_eq!(split_path("file.txt"), Some(("".into(), "file.txt".into())));
/// assert_eq!(split_path("/a/b/c.txt"), Some(("a/b".into(), "c.txt".into())));
/// assert_eq!(split_path(""), None);
/// assert_eq!(split_path("/"), None);
/// ```
#[deprecated(since = "2.0.0", note = "use hadris_path::split_path")]
pub fn split_path(path: &str) -> Option<(String, String)> {
    hadris_path::split_path(path)
}

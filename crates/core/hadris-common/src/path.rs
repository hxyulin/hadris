//! Path utilities shared across filesystem implementations.

extern crate alloc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

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
pub fn split_path(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }

    let filename = parts.last().unwrap().to_string();
    let dir_path = if parts.len() > 1 {
        parts[..parts.len() - 1].join("/")
    } else {
        String::new()
    };

    Some((dir_path, filename))
}

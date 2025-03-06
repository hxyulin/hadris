//! This module contains structures and functions for working with files.


/// Errors that can occur when working with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FileError {
    /// The file was not found, and when trying to create a new file, this error is returned if the
    /// parent directory does not exist.
    #[error("File is not open")]
    FileNotFound,
    /// The file already exists when trying to create a new file, if you want to overwrite an existing
    /// file, use the file flags
    /// TODO: Add flags
    #[error("File already exists")]
    FileAlreadyExists,
}

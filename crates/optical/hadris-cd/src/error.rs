//! Error types for hadris-cd

use hadris_io as io;

/// Errors that can occur during CD/DVD image creation
#[derive(Debug, thiserror::Error)]
pub enum CdError {
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// ISO creation error
    #[error("ISO error: {0}")]
    Iso(#[from] hadris_iso::write::IsoCreationError),

    /// UDF error
    #[error("UDF error: {0}")]
    Udf(#[from] hadris_udf::UdfError),

    /// Invalid file path
    #[error("Invalid file path: {0}")]
    InvalidPath(String),

    /// File not found in tree
    #[error("File not found: {0}")]
    FileNotFound(String),

    /// Directory not found in tree
    #[error("Directory not found: {0}")]
    DirectoryNotFound(String),

    /// Volume name too long
    #[error("Volume name too long (max {max} characters): {name}")]
    VolumeNameTooLong { name: String, max: usize },

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Result type for CD operations
pub type CdResult<T> = Result<T, CdError>;

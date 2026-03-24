//! Command implementations for hadris-udf CLI

mod create;
mod info;
mod ls;
mod tree;

pub use create::create;
pub use info::info;
pub use ls::ls;
pub use tree::tree;

use std::fs::File;

use hadris_udf::{UdfDir, UdfFs};

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Navigate into a directory path within the UDF image.
pub(super) fn navigate_to_path(udf: &UdfFs<File>, path: &str) -> Result<UdfDir> {
    let mut dir = udf.root_dir()?;
    for component in path.split('/').filter(|s| !s.is_empty()) {
        let icb = dir
            .entries()
            .find(|e| e.is_dir() && e.name() == component)
            .map(|e| e.icb)
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("directory not found: {component}").into()
            })?;
        dir = udf.read_directory(&icb)?;
    }
    Ok(dir)
}

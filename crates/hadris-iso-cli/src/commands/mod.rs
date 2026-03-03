//! Command implementations for hadris-iso CLI

mod cat;
mod create;
mod extract;
mod info;
mod ls;
mod mkisofs;
mod tree;
mod verify;

pub use cat::cat;
pub use create::create;
pub use extract::extract;
pub use info::info;
pub use ls::ls;
pub use mkisofs::mkisofs;
pub use tree::tree;
pub use verify::verify;

use std::io::{Read, Seek};

use hadris_iso::directory::DirectoryRef;
use hadris_iso::read::IsoImage;
use hadris_iso::write::options::FormatOptions;
use hadris_iso::write::{InputFiles, estimator};

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Strip the ";1" version suffix from an ISO filename.
fn clean_name(name_bytes: &[u8]) -> String {
    let name = String::from_utf8_lossy(name_bytes);
    if let Some(pos) = name.rfind(';') {
        name[..pos].to_string()
    } else {
        name.to_string()
    }
}

/// Navigate into a directory path within the ISO, returning the target DirectoryRef.
fn navigate_to_path<R: Read + Seek>(iso: &IsoImage<R>, path: &str) -> Result<DirectoryRef> {
    let root = iso.root_dir();
    let mut current = root.dir_ref();
    for component in path.split('/').filter(|s| !s.is_empty()) {
        let dir = iso.open_dir(current);
        let found = dir
            .entries()
            .filter_map(|e| e.ok())
            .find(|e| {
                let name = clean_name(e.name());
                name.eq_ignore_ascii_case(component) && e.is_directory()
            })
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("Directory not found: {}", component).into()
            })?;
        current = found.as_dir_ref(iso)?;
    }
    Ok(current)
}

/// Compute estimated size using the estimator API.
fn compute_estimated_size(input: &InputFiles, format_options: &FormatOptions) -> u64 {
    let estimate = estimator::estimate(input, format_options);
    estimate.minimum_bytes() + 1024 * 1024 // safety margin
}

/// Normalize a path to use forward slashes (ISO 9660 standard).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn count_files(input: &InputFiles) -> usize {
    fn count_recursive(files: &[hadris_iso::write::File]) -> usize {
        files
            .iter()
            .map(|f| match f {
                hadris_iso::write::File::File { .. } => 1,
                hadris_iso::write::File::Directory { children, .. } => {
                    1 + count_recursive(children)
                }
            })
            .sum()
    }
    count_recursive(&input.files)
}


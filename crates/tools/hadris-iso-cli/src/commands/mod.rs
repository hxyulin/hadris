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
use hadris_iso::file::EntryType;
use hadris_iso::read::{DirEntry, IsoImage};
use hadris_iso::write::options::IsoFormatOptions;
use hadris_iso::write::{InputEntry, InputEntryKind, InputTree, estimator};

pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Return the most capable name exposed by the image, without an ISO version suffix.
fn display_name(entry: &DirEntry, entry_type: EntryType) -> String {
    let name = if matches!(entry_type, EntryType::Joliet { .. }) && entry.rrip.is_none() {
        let units: Vec<u16> = entry
            .name()
            .chunks_exact(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            .collect();
        std::borrow::Cow::Owned(String::from_utf16_lossy(&units))
    } else {
        entry.display_name()
    };
    match name.rsplit_once(';') {
        Some((base, version))
            if !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            base.to_string()
        }
        _ => name.into_owned(),
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
            .find(|e| e.is_directory() && e.matches_name(component))
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("Directory not found: {component}").into()
            })?;
        current = found.as_dir_ref(iso)?;
    }
    Ok(current)
}

/// Compute estimated size using the estimator API.
fn compute_estimated_size(input: &InputTree, format_options: &IsoFormatOptions) -> u64 {
    let estimate = estimator::estimate_tree(input, format_options);
    estimate.minimum_bytes() + 1024 * 1024 // safety margin
}

/// Normalize a path to use forward slashes (ISO 9660 standard).
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn count_files(input: &InputTree) -> usize {
    fn count_recursive(files: &[InputEntry]) -> usize {
        files
            .iter()
            .map(|entry| match &entry.kind {
                InputEntryKind::Directory(children) => 1 + count_recursive(children),
                _ => 1,
            })
            .sum()
    }
    count_recursive(&input.entries)
}

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
use hadris_iso::susp::SystemUseIter;
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

/// Detect Rock Ridge support by reading the root directory's dot entry
/// and looking for SUSP SP + RRIP ER entries, following CE if needed.
fn detect_rock_ridge<R: Read + Seek>(iso: &IsoImage<R>) -> bool {
    let root = iso.root_dir();
    let dir = root.iter(iso);
    let mut entries = dir.entries();

    // The first entry should be the dot entry (current directory)
    let dot_entry = match entries.next() {
        Some(Ok(entry)) if entry.name() == b"\x00" => entry,
        _ => return false,
    };

    let su_data = dot_entry.system_use();
    if su_data.len() < 7 {
        return false;
    }

    let mut found_sp = false;
    let mut found_rrip_er = false;

    // Parse inline system use entries
    for field in SystemUseIter::new(su_data, 0) {
        match field {
            hadris_iso::susp::SystemUseField::SuspIdentifier(sp) => {
                if sp.is_valid() {
                    found_sp = true;
                }
            }
            hadris_iso::susp::SystemUseField::ExtensionReference(er) => {
                let id_start = 4usize;
                let id_end = id_start + er.identifier_len as usize;
                if id_end <= er.buf.len() {
                    let id = &er.buf[id_start..id_end];
                    if id == b"RRIP_1991A" {
                        found_rrip_er = true;
                    }
                }
            }
            hadris_iso::susp::SystemUseField::ContinuationArea(ce) => {
                let sector = ce.sector.read() as u64;
                let offset = ce.offset.read() as u64;
                let length = ce.length.read() as usize;
                if length > 0 {
                    let byte_pos = sector * 2048 + offset;
                    let mut buf = vec![0u8; length];
                    if iso.read_bytes_at(byte_pos, &mut buf).is_ok() {
                        for ce_field in SystemUseIter::new(&buf, 0) {
                            if let hadris_iso::susp::SystemUseField::ExtensionReference(er) =
                                ce_field
                            {
                                let id_start = 4usize;
                                let id_end = id_start + er.identifier_len as usize;
                                if id_end <= er.buf.len() {
                                    let id = &er.buf[id_start..id_end];
                                    if id == b"RRIP_1991A" {
                                        found_rrip_er = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    found_sp && found_rrip_er
}

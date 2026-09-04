use crate::{
    rrip::RripBuilder,
    write::{writer::DirectoryId, *},
};

pub fn system_time_seconds(value: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    value
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

pub fn read_input_directory_recursively(
    current_path: &std::path::Path,
) -> core::result::Result<Vec<InputEntry>, FileConversionError> {
    use alloc::string::ToString;
    let mut children = Vec::new();
    for entry in std::fs::read_dir(current_path)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| FileConversionError::InvalidUtf8Path(path.clone()))?
            .to_string();
        let fs_metadata: std::fs::Metadata = std::fs::symlink_metadata(&path)?;
        let file_type = fs_metadata.file_type();
        let metadata = InputMetadata::from_fs(fs_metadata);
        let kind = InputEntryKind::new(file_type, path)?;

        children.push(InputEntry {
            name: Arc::new(name),
            kind,
            metadata,
        });
    }
    children.sort_by_key(|entry| entry.name.to_ascii_lowercase());
    Ok(children)
}

/// Apply a deduplication suffix to a name, producing e.g. `READM_1.TXT;1`.
///
/// The suffix `_N` is inserted before the extension (and before any `;1` version
/// suffix). The basename is truncated if needed to stay within format limits.
pub fn apply_dedup_suffix(name: &[u8], n: usize, ty: EntryType) -> Vec<u8> {
    let suffix = alloc::format!("_{n}");

    match ty {
        EntryType::Joliet { .. } => apply_joliet_dedup_suffix(name, &suffix),
        _ => apply_iso_dedup_suffix(name, &suffix, ty),
    }
}

/// Applies a deduplication suffix to a Joliet (UTF‑16 BE) name.
///
/// Joliet names are UCS‑2 (UTF‑16 BE) encoded. This function finds the extension
/// (dot) position, inserts the suffix before it, and truncates to the maximum
/// Joliet name length (103 code units = 206 bytes).
fn apply_joliet_dedup_suffix(name: &[u8], suffix: &str) -> Vec<u8> {
    let suffix_u16: Vec<u8> = suffix
        .encode_utf16()
        .flat_map(|c| c.to_be_bytes())
        .collect();

    let mut dot_pos = None;
    let mut i = 0;
    while i + 1 < name.len() {
        if name[i] == 0x00 && name[i + 1] == 0x2E {
            dot_pos = Some(i);
            break; // Only the first dot matters
        }
        i += 2;
    }

    let (basename, ext) = match dot_pos {
        Some(pos) => (&name[..pos], &name[pos..]),
        None => (name, &[][..]),
    };

    // Max 206 bytes = 103 code units (UCS‑2)
    let max_basename = 206usize.saturating_sub(ext.len() + suffix_u16.len());
    let trunc_basename = &basename[..basename.len().min(max_basename) & !1];

    let mut result = Vec::with_capacity(trunc_basename.len() + suffix_u16.len() + ext.len());
    result.extend_from_slice(trunc_basename);
    result.extend_from_slice(&suffix_u16);
    result.extend_from_slice(ext);
    result
}

/// Applies a deduplication suffix to an ISO Level 1, 2, or 3 name.
///
/// ISO names are ASCII-based and may include a `;1` version suffix.
/// The suffix is inserted before the extension (and before `;1` if present).
fn apply_iso_dedup_suffix(name: &[u8], suffix: &str, ty: EntryType) -> Vec<u8> {
    let suffix_bytes = suffix.as_bytes();

    // Strip ";1" version suffix if present
    let (base_name, version) = if name.ends_with(b";1") {
        (&name[..name.len() - 2], &b";1"[..])
    } else {
        (name, &[][..])
    };

    // Find the extension separator (last dot)
    let dot_pos = base_name.iter().rposition(|&b| b == b'.');
    let (basename, ext) = match dot_pos {
        Some(pos) => (&base_name[..pos], &base_name[pos..]),
        None => (base_name, &[][..]),
    };

    // Determine max basename length based on ISO level
    let max_total = match ty {
        EntryType::Level1 { .. } => 8, // 8.3 format
        EntryType::Level2 { .. } => 30usize.saturating_sub(ext.len()),
        _ => 207usize.saturating_sub(ext.len() + version.len()), // Level 3
    };

    // Truncate basename to fit suffix
    let max_basename = max_total.saturating_sub(suffix_bytes.len());
    let trunc_basename = &basename[..basename.len().min(max_basename)];

    // Build result: basename + suffix + extension + version
    let mut result =
        Vec::with_capacity(trunc_basename.len() + suffix_bytes.len() + ext.len() + version.len());
    result.extend_from_slice(trunc_basename);
    result.extend_from_slice(suffix_bytes);
    result.extend_from_slice(ext);
    result.extend_from_slice(version);
    result
}

pub fn relocate_deep_directories(files: &mut WrittenFiles) {
    fn visit(
        dir: &mut WrittenDirectory,
        physical_depth: usize,
        physical_path_len: usize,
        moved: &mut Vec<WrittenDirectory>,
        internal_id: &mut usize,
    ) {
        let mut retained = Vec::with_capacity(dir.dirs.len());
        for mut child in core::mem::take(&mut dir.dirs) {
            let child_path_len = if physical_path_len == 0 {
                child.name.len()
            } else {
                physical_path_len + 1 + child.name.len()
            };
            if physical_depth + 1 > 8 || child_path_len > 255 {
                let target = child.id;
                let logical_parent = dir.id;
                let original_name = child.rrip_name.clone();
                child.name = Arc::new(alloc::format!("RRD{:06}", *internal_id));
                *internal_id += 1;
                child.relocation = DirectoryRelocation::Moved {
                    id: target,
                    logical_parent,
                };
                let relocated_path_len = "RR_MOVED".len() + 1 + child.name.len();
                visit(&mut child, 3, relocated_path_len, moved, internal_id);
                moved.push(child);

                let mut placeholder = WrittenDirectory::new(original_name);
                placeholder.relocation = DirectoryRelocation::Placeholder { target };
                retained.push(placeholder);
            } else {
                visit(
                    &mut child,
                    physical_depth + 1,
                    child_path_len,
                    moved,
                    internal_id,
                );
                retained.push(child);
            }
        }
        dir.dirs = retained;
    }

    let root = files.get_mut(&files.root_dir());
    let mut moved = Vec::new();
    let mut internal_id = 1;
    visit(root, 1, 0, &mut moved, &mut internal_id);
    if moved.is_empty() {
        return;
    }

    let occupied = root
        .dirs
        .iter()
        .map(|directory| directory.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut relocation_name = String::from("RR_MOVED");
    let mut suffix = 1;
    while occupied.contains(relocation_name.as_str()) {
        relocation_name = alloc::format!("RR_MOVED_{suffix}");
        suffix += 1;
    }
    let mut relocation_dir = WrittenDirectory::new(Arc::new(relocation_name));
    relocation_dir.id = usize::MAX;
    relocation_dir.dirs = moved;
    root.dirs.insert(0, relocation_dir);
}

/// Generates a deterministic GUID from a string (simple hash-based).
pub fn generate_guid_from_string(s: &str) -> Guid {
    // Simple FNV-1a hash to generate a deterministic GUID
    let mut hash1: u64 = 0xcbf29ce484222325;
    let mut hash2: u64 = 0x100000001b3;

    for byte in s.bytes() {
        hash1 ^= byte as u64;
        hash1 = hash1.wrapping_mul(0x100000001b3);
        hash2 ^= byte as u64;
        hash2 = hash2.wrapping_mul(0xcbf29ce484222325);
    }

    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&hash1.to_le_bytes());
    bytes[8..16].copy_from_slice(&hash2.to_le_bytes());

    // Set version 4 (random) and variant bits
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // Version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // Variant 1

    Guid::from_bytes(bytes)
}

/// Compute the available system use space in a DirectoryRecord given
/// the ISO name length. The record is 256 bytes max; the fixed header
/// is 33 bytes, followed by the name (padded to even).
pub const fn available_su_space(iso_name_len: usize) -> usize {
    let used = (33 + iso_name_len + 1) & !1; // pad to even boundary
    256usize.saturating_sub(used)
}

/// Build complete RRIP entries for a directory record.
///
/// Entries are ordered by priority (most important first, largest last),
/// so that `build_split` keeps the important ones inline and overflows
/// the rest via a CE pointer.
pub fn rrip_datetime(timestamp: Option<i64>, fallback: &[u8; 7]) -> [u8; 7] {
    use chrono::{Datelike, Timelike};
    let Some(timestamp) = timestamp.and_then(|value| chrono::DateTime::from_timestamp(value, 0))
    else {
        return *fallback;
    };
    [
        (timestamp.year() - 1900).clamp(0, 255) as u8,
        timestamp.month() as u8,
        timestamp.day() as u8,
        timestamp.hour() as u8,
        timestamp.minute() as u8,
        timestamp.second() as u8,
        0,
    ]
}

/// Adds common RRIP fields (PX, TF) to a builder.
///
/// Includes POSIX attributes (mode, nlink, uid, gid, inode) and
/// timestamps if preservation is enabled.
#[allow(clippy::too_many_arguments)]
pub fn add_posix_attributes(
    options: &RripOptions,
    fallback_time: &RripTime,
    inode: u32,
    builder: &mut RripBuilder,
    metadata: InputMetadata,
    type_mode: u32,
    default_permissions: u32,
    nlink: u32,
) {
    let permissions = if options.preserve_permissions {
        metadata.mode.unwrap_or(default_permissions)
    } else {
        default_permissions
    };
    let (uid, gid) = if options.preserve_ownership {
        (metadata.uid.unwrap_or(0), metadata.gid.unwrap_or(0))
    } else {
        (0, 0)
    };

    builder.add_px(type_mode | permissions, nlink, uid, gid, inode);

    if options.preserve_timestamps {
        let modified = rrip_datetime(metadata.modified, fallback_time);
        let accessed = rrip_datetime(metadata.accessed, fallback_time);
        // Emit the creation timestamp when the input carries one; in-memory
        // entries without a creation time keep the modify/access-only form.
        let created = metadata
            .created
            .map(|created| rrip_datetime(Some(created), fallback_time));
        builder.add_tf(created.as_ref(), &modified, &accessed);
    }
}

pub fn build_rrip_entries(
    kind: RripEntryKind<'_>,
    inode: u32,
    options: &RripOptions,
    fallback_time: &RripTime,
) -> RripBuilder {
    let mut builder = RripBuilder::new();

    match &kind {
        RripEntryKind::RootDot { metadata, nlink } => {
            builder.add_sp(0);
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                0o040000,
                0o755,
                *nlink,
            );
            builder.add_nm_current();
            builder.add_rrip_er(); // full ER, last (largest)
        }
        RripEntryKind::RootDotDot { metadata, nlink } => {
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                0o040000,
                0o755,
                *nlink,
            );
            builder.add_nm_parent();
        }
        RripEntryKind::Dot { metadata, nlink } => {
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                0o040000,
                0o755,
                *nlink,
            );
            builder.add_nm_current();
        }
        RripEntryKind::DotDot { metadata, nlink } => {
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                0o040000,
                0o755,
                *nlink,
            );
            builder.add_nm_parent();
        }
        RripEntryKind::Directory {
            original_name,
            metadata,
            nlink,
        } => {
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                0o040000,
                0o755,
                *nlink,
            );
            builder.add_nm(original_name.as_bytes());
        }
        RripEntryKind::Entry {
            original_name,
            metadata,
            kind,
        } => {
            let (type_mode, default_permissions) = match kind {
                InputEntryKind::File(_) | InputEntryKind::Source(_) => (0o100000, 0o644),
                #[cfg(test)]
                InputEntryKind::TestFile { .. } => (0o100000, 0o644),
                InputEntryKind::Symlink(_) => (0o120000, 0o777),
                InputEntryKind::CharacterDevice { .. } => (0o020000, 0o600),
                InputEntryKind::BlockDevice { .. } => (0o060000, 0o600),
                InputEntryKind::Directory(_) => unreachable!(),
            };
            add_posix_attributes(
                options,
                fallback_time,
                inode,
                &mut builder,
                *metadata,
                type_mode,
                default_permissions,
                1,
            );
            builder.add_nm(original_name.as_bytes());
            match kind {
                InputEntryKind::Symlink(target) => {
                    builder.add_sl(target);
                }
                InputEntryKind::CharacterDevice { major, minor }
                | InputEntryKind::BlockDevice { major, minor } => {
                    builder.add_pn(*major, *minor);
                }
                _ => {}
            }
        }
    }

    builder
}

pub struct MovedDirectory {
    #[allow(dead_code)]
    pub id: usize,
    pub logical_parent: usize,
    pub entries: BTreeMap<EntryType, DirectoryRef>,
}

impl MovedDirectory {
    pub const fn new(
        id: usize,
        logical_parent: usize,
        entries: BTreeMap<EntryType, DirectoryRef>,
    ) -> Self {
        Self {
            id,
            logical_parent,
            entries,
        }
    }

    pub fn collect(directory: &WrittenDirectory) -> Vec<MovedDirectory> {
        let mut output = vec![];
        collect_moved_recursive(directory, &mut output);
        output
    }
}

fn collect_moved_recursive(directory: &WrittenDirectory, output: &mut Vec<MovedDirectory>) {
    if let DirectoryRelocation::Moved { id, logical_parent } = directory.relocation {
        output.push(MovedDirectory::new(
            id,
            logical_parent,
            directory.entries.clone(),
        ));
    }
    for child in &directory.dirs {
        collect_moved_recursive(child, output);
    }
}

/// Compute the sector span `records` will occupy when written at byte
/// position `pos`: returns (start sector, size in sectors). Mirrors the
/// write logic in [`Self::write_directory_records`] exactly.
pub fn layout_directory_records(
    pos: u64,
    sector_size: u64,
    records: &PendingRecords,
) -> (u64, u64) {
    let align = |pos: u64| (pos + sector_size - 1) & !(sector_size - 1);
    let start = align(pos);
    let mut pos = start;
    for record in records.iter() {
        let record_size = DirectoryRecord::new(
            &record.name,
            &record.split.inline,
            record.dir_ref,
            record.flags,
        )
        .size() as u64;
        let remaining = sector_size - pos % sector_size;
        if record_size > remaining {
            pos += remaining;
        }
        pos += record_size;
    }
    let end = align(pos);
    (start / sector_size, (end - start) / sector_size)
}

// Pre-order: parents before children. libarchive (bsdtar) scans
// directories as a stream and ignores a directory whose extent is
// lower than its current position, so extents must ascend from
// parents to children.
pub fn collect_preorder(
    files: &WrittenFiles,
    id: &DirectoryId,
) -> (Vec<DirectoryId>, Vec<(DirectoryId, usize)>) {
    let mut order = vec![];
    collect_preorder_recursive(files, id, &mut order);

    let mut file_order = Vec::new();
    for directory_id in &order {
        let dir = files.get(directory_id);
        for (index, file) in dir.files.iter().enumerate() {
            if file.kind.file_len().is_some_and(|len| len > 0) {
                file_order.push((directory_id.clone(), index));
            }
        }
    }

    (order, file_order)
}

fn collect_preorder_recursive(
    files: &WrittenFiles,
    id: &writer::DirectoryId,
    output: &mut Vec<writer::DirectoryId>,
) {
    output.push(id.clone());
    let dir = files.get(id);
    for (index, child) in dir.dirs.iter().enumerate() {
        if matches!(child.relocation, DirectoryRelocation::Placeholder { .. }) {
            continue;
        }
        let mut child_id = id.clone();
        child_id.push(index);
        collect_preorder_recursive(files, &child_id, output);
    }
}

pub const fn alignment_requires_materialization(
    current_position: u64,
    aligned_position: u64,
) -> bool {
    aligned_position > current_position
}

pub fn part_io_error(err: hadris_part::Error) -> io::Error {
    match err {
        hadris_part::Error::Io(err) => err,
        _ => io::Error::new(
            io::ErrorKind::InvalidData,
            "failed to write partition table",
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::{read::PathSeparator, write::utils::apply_dedup_suffix};

    use super::*;
    use alloc::vec;

    #[test]
    fn alignment_only_materializes_new_padding() {
        assert!(!alignment_requires_materialization(2048, 2048));
        assert!(alignment_requires_materialization(2047, 2048));
    }

    #[test]
    fn test_depth_first_tree_walk_iterator() {
        // Define a test file hierarchy
        let file_a = InputEntry::file("root/dir1/fileA.txt", Vec::new());
        let file_b = InputEntry::file("root/dir1/fileB.txt", Vec::new());
        let file_c = InputEntry::file("root/fileC.txt", Vec::new());
        let file_d = InputEntry::file("root/dir2/fileD.txt", Vec::new());
        let file_e = InputEntry::file("root/dir2/subdir/fileE.txt", Vec::new());

        let subdir_node = InputEntry::directory("root/dir2/subdir", vec![file_e.clone()]);

        let dir1_node = InputEntry::directory("root/dir1", vec![file_a.clone(), file_b.clone()]);

        let dir2_node = InputEntry::directory(
            "root/dir2",
            vec![
                file_d.clone(),
                subdir_node.clone(), // Subdirectory
            ],
        );

        let root_level_files = vec![dir1_node.clone(), file_c.clone(), dir2_node.clone()];

        let input_tree = InputTree::new(PathSeparator::ForwardSlash, root_level_files);

        // Create the iterator
        let walker = FileTreeWalker::new(&input_tree);

        // Define the expected sequence of events (depth-first, pre-order for Enter, post-order for Exit)
        let expected_sequence = vec![
            TreeWalkerItem::EnterDirectory(&dir1_node),   // Enter dir1
            TreeWalkerItem::File(&file_a),                // Process fileA
            TreeWalkerItem::File(&file_b),                // Process fileB
            TreeWalkerItem::ExitDirectory(&dir1_node),    // Exit dir1
            TreeWalkerItem::File(&file_c),                // Process fileC
            TreeWalkerItem::EnterDirectory(&dir2_node),   // Enter dir2
            TreeWalkerItem::File(&file_d),                // Process fileD
            TreeWalkerItem::EnterDirectory(&subdir_node), // Enter subdir
            TreeWalkerItem::File(&file_e),                // Process fileE
            TreeWalkerItem::ExitDirectory(&subdir_node),  // Exit subdir
            TreeWalkerItem::ExitDirectory(&dir2_node),    // Exit dir2
        ];

        // Collect all items from the iterator
        let actual_sequence: Vec<TreeWalkerItem> = walker.collect();

        // Assert that the actual sequence matches the expected sequence
        assert_eq!(actual_sequence, expected_sequence);
    }

    #[test]
    fn test_dedup_suffix_l1_with_ext() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"README.TXT;1", 1, ty);
        assert_eq!(result, b"README_1.TXT;1");
    }

    #[test]
    fn test_dedup_suffix_l1_no_ext() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"FILENAME;1", 1, ty);
        assert_eq!(result, b"FILENA_1;1");
    }

    #[test]
    fn test_dedup_suffix_l2() {
        let ty = EntryType::Level2 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"LONGFILENAME.EXT;1", 2, ty);
        assert_eq!(result, b"LONGFILENAME_2.EXT;1");
    }

    #[test]
    fn test_dedup_suffix_l3_no_version() {
        let ty = EntryType::Level3 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"README.TXT", 1, ty);
        assert_eq!(result, b"README_1.TXT");
    }

    #[test]
    fn test_dedup_suffix_distinct() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let r1 = apply_dedup_suffix(b"README.TXT;1", 1, ty);
        let r2 = apply_dedup_suffix(b"README.TXT;1", 2, ty);
        let r3 = apply_dedup_suffix(b"README.TXT;1", 3, ty);
        assert_ne!(r1, r2);
        assert_ne!(r2, r3);
        assert_ne!(r1, r3);
    }
}

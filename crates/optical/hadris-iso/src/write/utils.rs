use alloc::{collections::BTreeSet, string::ToString};

use crate::{
    file::EntryType,
    rrip::RripBuilder,
    write::{writer::DirectoryId, *},
};

const RECOGNIZED_RELOCATION_NAMES: [&str; 2] = ["rr_moved", ".rr_moved"];

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

pub fn relocate_deep_directories(files: &mut WrittenFiles, primary: EntryType) -> io::Result<()> {
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
                let relocated_path_len = ".rr_moved".len() + 1 + child.name.len();
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
        return Ok(());
    }

    place_relocated_directories(root, moved, primary)
}

fn is_recognized_relocation_name(name: &str) -> bool {
    RECOGNIZED_RELOCATION_NAMES.contains(&name)
}

fn iso_directory_identifier(ty: EntryType, name: &str) -> Vec<u8> {
    ty.convert_directory_name(name).as_bytes().to_vec()
}

fn name_taken(root: &WrittenDirectory, name: &str) -> bool {
    root.dirs.iter().any(|entry| entry.name.as_str() == name)
        || root.files.iter().any(|entry| entry.name.as_str() == name)
}

/// libarchive binds RE entries to the first root directory whose Rock Ridge
/// name is `rr_moved` or `.rr_moved`, in ISO File Identifier order. Create a
/// free recognized name only when it would sort first; otherwise reuse the
/// existing directory that libarchive will select.
fn place_relocated_directories(
    root: &mut WrittenDirectory,
    mut moved: Vec<WrittenDirectory>,
    primary: EntryType,
) -> io::Result<()> {
    let existing_dirs: Vec<usize> = root
        .dirs
        .iter()
        .enumerate()
        .filter(|(_, dir)| is_recognized_relocation_name(dir.name.as_str()))
        .map(|(index, _)| index)
        .collect();
    let first_existing_iso = existing_dirs
        .iter()
        .map(|&index| iso_directory_identifier(primary, root.dirs[index].name.as_str()))
        .min();
    let created_name = RECOGNIZED_RELOCATION_NAMES.into_iter().find(|name| {
        !name_taken(root, name)
            && first_existing_iso
                .as_ref()
                .is_none_or(|existing| iso_directory_identifier(primary, name) < *existing)
    });

    if let Some(name) = created_name {
        let mut relocation_dir = WrittenDirectory::new(Arc::new(String::from(name)));
        relocation_dir.id = usize::MAX;
        assign_unique_relocated_names(&relocation_dir, &mut moved);
        relocation_dir.dirs = moved;
        root.dirs.insert(0, relocation_dir);
        return Ok(());
    }

    let reuse_index = existing_dirs
        .into_iter()
        .min_by_key(|&index| iso_directory_identifier(primary, root.dirs[index].name.as_str()))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Rock Ridge relocation requires an available rr_moved or .rr_moved directory",
            )
        })?;
    if reuse_index != 0 {
        let directory = root.dirs.remove(reuse_index);
        root.dirs.insert(0, directory);
    }
    assign_unique_relocated_names(&root.dirs[0], &mut moved);
    let mut dirs = moved;
    dirs.append(&mut root.dirs[0].dirs);
    root.dirs[0].dirs = dirs;
    Ok(())
}

fn assign_unique_relocated_names(container: &WrittenDirectory, moved: &mut [WrittenDirectory]) {
    let mut used = BTreeSet::new();
    for child in &container.dirs {
        used.insert(child.name.to_string());
    }
    for file in &container.files {
        used.insert(file.name.to_string());
    }
    let mut next = 1usize;
    for child in moved {
        if used.insert(child.name.to_string()) {
            continue;
        }
        loop {
            let candidate = alloc::format!("RRD{next:06}");
            next += 1;
            if used.insert(candidate.clone()) {
                child.name = Arc::new(candidate);
                break;
            }
        }
    }
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
                InputEntryKind::File(_) => (0o100000, 0o644),
                #[cfg(feature = "unstable-streaming")]
                InputEntryKind::Source(_) => (0o100000, 0o644),
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

/// Converts a sector count to the 32-bit field of an MBR partition entry or the
/// ISO 9660 volume space size, failing with `InvalidInput` instead of truncating.
pub fn checked_sector_count(sectors: u64, message: &'static str) -> io::Result<u32> {
    u32::try_from(sectors).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, message))
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

    fn nested_written(depth: usize, extra: Vec<InputEntry>) -> WrittenFiles {
        let mut children = vec![InputEntry::file("leaf.txt", Vec::new())];
        for level in (1..=depth).rev() {
            children = vec![InputEntry::directory(format!("level{level}"), children)];
        }
        children.extend(extra);
        let tree = InputTree::new(PathSeparator::ForwardSlash, children);
        let mut files = WrittenFiles::new();
        FileTreeWalker::new(&tree).walk(&mut files);
        files
    }

    fn primary(lowercase: bool) -> EntryType {
        EntryType::Level1 {
            supports_lowercase: lowercase,
            supports_rrip: true,
        }
    }

    fn root_dir_names(files: &WrittenFiles) -> Vec<String> {
        files
            .get(&files.root_dir())
            .dirs
            .iter()
            .map(|dir| dir.name.to_string())
            .collect()
    }

    fn container<'a>(files: &'a WrittenFiles, name: &str) -> &'a WrittenDirectory {
        files
            .get(&files.root_dir())
            .dirs
            .iter()
            .find(|dir| dir.name.as_str() == name)
            .unwrap()
    }

    #[test]
    fn uppercase_iso_names_sort_rr_moved_before_dot_name() {
        let ty = primary(false);
        assert!(
            iso_directory_identifier(ty, "rr_moved") < iso_directory_identifier(ty, ".rr_moved")
        );
    }

    #[test]
    fn lowercase_iso_names_sort_dot_name_before_rr_moved() {
        let ty = primary(true);
        assert!(
            iso_directory_identifier(ty, ".rr_moved") < iso_directory_identifier(ty, "rr_moved")
        );
    }

    #[test]
    fn reuses_user_rr_moved_directory_when_it_sorts_first() {
        let mut files = nested_written(
            9,
            vec![InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            )],
        );
        relocate_deep_directories(&mut files, primary(false)).unwrap();
        assert_eq!(root_dir_names(&files)[0], "rr_moved");
        assert!(!root_dir_names(&files).contains(&".rr_moved".to_string()));
        let rr_moved = container(&files, "rr_moved");
        assert!(
            rr_moved
                .dirs
                .iter()
                .any(|dir| matches!(dir.relocation, DirectoryRelocation::Moved { .. }))
        );
        assert!(
            rr_moved
                .files
                .iter()
                .any(|file| file.name.as_str() == "user.txt")
        );
    }

    #[test]
    fn creates_rr_moved_when_only_dot_name_directory_exists() {
        let mut files = nested_written(
            9,
            vec![InputEntry::directory(
                ".rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            )],
        );
        relocate_deep_directories(&mut files, primary(false)).unwrap();
        let names = root_dir_names(&files);
        assert_eq!(names[0], "rr_moved");
        assert!(names.contains(&".rr_moved".to_string()));
        assert!(
            container(&files, "rr_moved")
                .dirs
                .iter()
                .any(|dir| matches!(dir.relocation, DirectoryRelocation::Moved { .. }))
        );
        assert!(
            container(&files, ".rr_moved")
                .files
                .iter()
                .any(|file| file.name.as_str() == "user.txt")
        );
    }

    #[test]
    fn reuses_iso_first_directory_when_both_recognized_names_exist() {
        let mut files = nested_written(
            9,
            vec![
                InputEntry::directory(
                    "rr_moved",
                    vec![InputEntry::file("plain.txt", b"plain".to_vec())],
                ),
                InputEntry::directory(
                    ".rr_moved",
                    vec![InputEntry::file("dot.txt", b"dot".to_vec())],
                ),
            ],
        );
        relocate_deep_directories(&mut files, primary(false)).unwrap();
        let names = root_dir_names(&files);
        assert_eq!(names[0], "rr_moved");
        assert_eq!(names.iter().filter(|name| *name == "rr_moved").count(), 1);
        assert_eq!(names.iter().filter(|name| *name == ".rr_moved").count(), 1);
        let rr_moved = container(&files, "rr_moved");
        assert!(
            rr_moved
                .dirs
                .iter()
                .any(|dir| matches!(dir.relocation, DirectoryRelocation::Moved { .. }))
        );
        assert!(
            rr_moved
                .files
                .iter()
                .any(|file| file.name.as_str() == "plain.txt")
        );
        assert!(
            container(&files, ".rr_moved")
                .files
                .iter()
                .any(|file| file.name.as_str() == "dot.txt")
        );
    }

    #[test]
    fn lowercase_creates_dot_name_ahead_of_user_rr_moved() {
        let mut files = nested_written(
            9,
            vec![InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            )],
        );
        relocate_deep_directories(&mut files, primary(true)).unwrap();
        let names = root_dir_names(&files);
        assert_eq!(names[0], ".rr_moved");
        assert!(names.contains(&"rr_moved".to_string()));
        assert!(
            container(&files, ".rr_moved")
                .dirs
                .iter()
                .any(|dir| matches!(dir.relocation, DirectoryRelocation::Moved { .. }))
        );
        assert!(
            container(&files, "rr_moved")
                .files
                .iter()
                .any(|file| file.name.as_str() == "user.txt")
        );
    }

    #[test]
    fn avoids_physical_name_collisions_inside_reused_container() {
        let mut files = nested_written(
            9,
            vec![InputEntry::directory(
                "rr_moved",
                vec![
                    InputEntry::file("RRD000001", b"taken".to_vec()),
                    InputEntry::directory("RRD000002", Vec::new()),
                ],
            )],
        );
        relocate_deep_directories(&mut files, primary(false)).unwrap();
        let rr_moved = container(&files, "rr_moved");
        let moved_names: Vec<_> = rr_moved
            .dirs
            .iter()
            .filter(|dir| matches!(dir.relocation, DirectoryRelocation::Moved { .. }))
            .map(|dir| dir.name.to_string())
            .collect();
        assert!(!moved_names.is_empty());
        assert!(
            !moved_names
                .iter()
                .any(|name| name == "RRD000001" || name == "RRD000002")
        );
        assert!(
            rr_moved
                .files
                .iter()
                .any(|file| file.name.as_str() == "RRD000001")
        );
        assert!(
            rr_moved
                .dirs
                .iter()
                .any(|dir| dir.name.as_str() == "RRD000002"
                    && matches!(dir.relocation, DirectoryRelocation::None))
        );
    }

    #[test]
    fn rejects_relocation_when_both_recognized_names_are_files() {
        let mut files = nested_written(
            9,
            vec![
                InputEntry::file("rr_moved", b"user".to_vec()),
                InputEntry::file(".rr_moved", b"user".to_vec()),
            ],
        );
        let error = relocate_deep_directories(&mut files, primary(false)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("available rr_moved or .rr_moved directory")
        );
    }
}

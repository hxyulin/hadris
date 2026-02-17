use std::fs::File;
use std::io::{self, BufReader, Read, Seek};

use hadris_iso::directory::FileFlags;
use hadris_iso::read::IsoImage;
use hadris_iso::susp::SystemUseIter;
use hadris_iso::types::Endian;
use hadris_iso::volume::VolumeDescriptor;

use crate::args::VerifyArgs;

use super::{Result, detect_rock_ridge};

// ── Verify types ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueSeverity {
    Error,
    Warning,
}

struct VerifyIssue {
    severity: IssueSeverity,
    message: String,
}

impl VerifyIssue {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Error,
            message: message.into(),
        }
    }

    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            message: message.into(),
        }
    }
}

// ── Verify sub-checks ──

fn check_volume_descriptors<R: Read + Seek>(
    iso: &IsoImage<R>,
    verbose: bool,
) -> (Vec<VerifyIssue>, bool, Option<u32>) {
    let mut issues = Vec::new();
    let mut found_pvd = false;
    let mut found_terminator = false;
    let mut boot_catalog_sector = None;

    for vd in iso.read_volume_descriptors() {
        match vd {
            Ok(VolumeDescriptor::Primary(_)) => {
                found_pvd = true;
                if verbose {
                    println!("  Found Primary Volume Descriptor");
                }
            }
            Ok(VolumeDescriptor::End(_)) => {
                found_terminator = true;
                if verbose {
                    println!("  Found Volume Descriptor Set Terminator");
                }
            }
            Ok(VolumeDescriptor::BootRecord(br)) => {
                boot_catalog_sector = Some(br.catalog_ptr.get());
                if verbose {
                    println!(
                        "  Found Boot Record (catalog sector: {})",
                        br.catalog_ptr.get()
                    );
                }
            }
            Ok(VolumeDescriptor::Supplementary(_)) => {
                if verbose {
                    println!("  Found Supplementary Volume Descriptor");
                }
            }
            Ok(VolumeDescriptor::Unknown(u)) => {
                if verbose {
                    println!("  Found Unknown Volume Descriptor (type {:?})", u);
                }
            }
            Err(e) => {
                issues.push(VerifyIssue::error(format!(
                    "Error reading volume descriptor: {}",
                    e
                )));
            }
        }
    }

    if !found_pvd {
        issues.push(VerifyIssue::error("Missing Primary Volume Descriptor"));
    }
    if !found_terminator {
        issues.push(VerifyIssue::error(
            "Missing Volume Descriptor Set Terminator",
        ));
    }

    (issues, found_pvd, boot_catalog_sector)
}

fn check_root_directory<R: Read + Seek>(iso: &IsoImage<R>, verbose: bool) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let root = iso.root_dir();
    let mut file_count = 0u64;
    let mut dir_count = 0u64;

    for entry in root.iter(iso).entries() {
        match entry {
            Ok(e) => {
                let flags = FileFlags::from_bits_truncate(e.header().flags);
                if flags.contains(FileFlags::DIRECTORY) {
                    dir_count += 1;
                } else {
                    file_count += 1;
                }
            }
            Err(e) => {
                issues.push(VerifyIssue::error(format!(
                    "Error reading root directory entry: {}",
                    e
                )));
            }
        }
    }

    if verbose {
        println!("  Files in root: {}", file_count);
        println!("  Directories in root: {}", dir_count);
    }

    issues
}

fn check_volume_size<R: Read + Seek>(
    iso: &IsoImage<R>,
    file_size: u64,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let pvd = iso.read_pvd();
    let declared_size = pvd.volume_space_size.read() as u64 * 2048;

    if verbose {
        println!(
            "  Volume size: {} bytes (declared), {} bytes (file)",
            declared_size, file_size
        );
    }

    if declared_size > file_size {
        issues.push(VerifyIssue::error(format!(
            "Volume declares {} bytes but file is only {} bytes (truncated image)",
            declared_size, file_size
        )));
    } else if file_size > declared_size + 32 * 1024 {
        issues.push(VerifyIssue::warning(format!(
            "File size ({}) exceeds declared volume size ({}) by {} bytes (common with hybrid boot images)",
            file_size,
            declared_size,
            file_size - declared_size
        )));
    }

    issues
}

fn check_path_table_consistency<R: Read + Seek>(
    iso: &IsoImage<R>,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let pt = iso.path_table();
    let entries: Vec<_> = match pt.entries(iso).collect::<std::result::Result<Vec<_>, _>>() {
        Ok(e) => e,
        Err(e) => {
            issues.push(VerifyIssue::error(format!(
                "Failed to read path table: {}",
                e
            )));
            return issues;
        }
    };

    let total = entries.len();
    if verbose {
        println!("  Path table entries: {}", total);
    }

    if total == 0 {
        issues.push(VerifyIssue::error("Path table is empty (no root entry)"));
        return issues;
    }

    // Root should have parent_index == 1
    if entries[0].parent_index != 1 {
        issues.push(VerifyIssue::error(format!(
            "Root path table entry has parent_index {} (expected 1)",
            entries[0].parent_index
        )));
    }

    for (i, entry) in entries.iter().enumerate() {
        let idx = i + 1; // path table is 1-indexed
        let parent = entry.parent_index as usize;
        if parent < 1 || parent > total {
            issues.push(VerifyIssue::error(format!(
                "Path table entry {} has invalid parent_index {} (valid range: 1..{})",
                idx, parent, total
            )));
            continue;
        }

        // Try to open the directory at this LBA
        let dir_ref = hadris_iso::directory::DirectoryRef {
            extent: hadris_iso::io::LogicalSector(entry.parent_lba as usize),
            size: 2048, // minimum; actual size may differ but this validates readability
        };
        let dir = iso.open_dir(dir_ref);
        if dir.entries().next().is_none() && entry.parent_lba != 0 {
            issues.push(VerifyIssue::warning(format!(
                "Path table entry {} (LBA {}) could not be read as a directory",
                idx, entry.parent_lba
            )));
        }
    }

    issues
}

fn check_extent_bounds<R: Read + Seek>(
    iso: &IsoImage<R>,
    file_size: u64,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let pvd = iso.read_pvd();
    let volume_size = pvd.volume_space_size.read() as u64 * 2048;

    fn walk_dir<R: Read + Seek>(
        iso: &IsoImage<R>,
        dir_ref: hadris_iso::directory::DirectoryRef,
        volume_size: u64,
        file_size: u64,
        issues: &mut Vec<VerifyIssue>,
        depth: usize,
    ) {
        if depth > 256 {
            issues.push(VerifyIssue::error(
                "Directory nesting exceeds 256 levels (possible loop in image)",
            ));
            return;
        }

        let dir = iso.open_dir(dir_ref);
        for entry in dir.entries() {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    issues.push(VerifyIssue::error(format!(
                        "Error reading directory entry at depth {}: {}",
                        depth, e
                    )));
                    continue;
                }
            };

            if entry.is_special() {
                continue;
            }

            let extent = entry.header().extent.read() as u64;
            let data_len = entry.header().data_len.read() as u64;
            if extent == 0 && data_len == 0 {
                continue; // zero-size file
            }

            let end_byte = extent * 2048 + data_len;
            if end_byte > volume_size {
                let name = String::from_utf8_lossy(entry.name());
                issues.push(VerifyIssue::error(format!(
                    "Entry '{}' extent end ({}) exceeds volume size ({})",
                    name, end_byte, volume_size
                )));
            }
            if end_byte > file_size {
                let name = String::from_utf8_lossy(entry.name());
                issues.push(VerifyIssue::error(format!(
                    "Entry '{}' extent end ({}) exceeds file size ({})",
                    name, end_byte, file_size
                )));
            }

            if entry.is_directory()
                && let Ok(child_ref) = entry.as_dir_ref(iso)
            {
                walk_dir(iso, child_ref, volume_size, file_size, issues, depth + 1);
            }
        }
    }

    if verbose {
        println!("  Checking extent bounds...");
    }

    let root = iso.root_dir();
    walk_dir(iso, root.dir_ref(), volume_size, file_size, &mut issues, 0);

    issues
}

fn check_boot_catalog<R: Read + Seek>(
    iso: &IsoImage<R>,
    catalog_sector: u32,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let byte_pos = catalog_sector as u64 * 2048;
    let mut buf = [0u8; 32];

    if let Err(e) = iso.read_bytes_at(byte_pos, &mut buf) {
        issues.push(VerifyIssue::error(format!(
            "Failed to read boot catalog at sector {}: {}",
            catalog_sector, e
        )));
        return issues;
    }

    let mut cursor = io::Cursor::new(&buf[..]);
    let validation = match hadris_iso::boot::BootValidationEntry::parse(&mut cursor) {
        Ok(v) => v,
        Err(e) => {
            issues.push(VerifyIssue::error(format!(
                "Failed to parse boot catalog validation entry: {}",
                e
            )));
            return issues;
        }
    };

    if validation.header_id != 0x01 {
        issues.push(VerifyIssue::error(format!(
            "Boot catalog validation entry has header_id {:#x} (expected 0x01)",
            validation.header_id
        )));
    }
    if validation.key != [0x55, 0xAA] {
        issues.push(VerifyIssue::error(format!(
            "Boot catalog validation entry has key {:?} (expected [0x55, 0xAA])",
            validation.key
        )));
    }

    let stored_checksum = validation.checksum.get();
    let calculated = validation.calculate_checksum();
    if stored_checksum != calculated {
        issues.push(VerifyIssue::error(format!(
            "Boot catalog checksum mismatch: stored {:#06x}, calculated {:#06x}",
            stored_checksum, calculated
        )));
    }

    if verbose && issues.is_empty() {
        println!("  Boot catalog validation passed");
    }

    issues
}

fn check_rrip_fields<R: Read + Seek>(iso: &IsoImage<R>, verbose: bool) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let pvd = iso.read_pvd();
    let volume_sectors = pvd.volume_space_size.read();

    fn walk_rrip<R: Read + Seek>(
        iso: &IsoImage<R>,
        dir_ref: hadris_iso::directory::DirectoryRef,
        volume_sectors: u32,
        issues: &mut Vec<VerifyIssue>,
        depth: usize,
    ) {
        if depth > 256 {
            return;
        }

        let dir = iso.open_dir(dir_ref);
        for entry in dir.entries() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let su = entry.system_use();
            if su.is_empty() {
                if !entry.is_special()
                    && entry.is_directory()
                    && let Ok(child_ref) = entry.as_dir_ref(iso)
                {
                    walk_rrip(iso, child_ref, volume_sectors, issues, depth + 1);
                }
                continue;
            }

            let is_dir =
                FileFlags::from_bits_truncate(entry.header().flags).contains(FileFlags::DIRECTORY);

            for field in SystemUseIter::new(su, 0) {
                match &field {
                    hadris_iso::susp::SystemUseField::PosixAttributes(px) => {
                        let mode = px.file_mode.read();
                        let file_type = mode & 0o170000;
                        if is_dir && file_type != 0o040000 && file_type != 0 {
                            let name = String::from_utf8_lossy(entry.name());
                            issues.push(VerifyIssue::warning(format!(
                                "PX mode type {:#o} doesn't match directory flag for '{}'",
                                file_type, name
                            )));
                        } else if !is_dir && file_type == 0o040000 {
                            let name = String::from_utf8_lossy(entry.name());
                            issues.push(VerifyIssue::warning(format!(
                                "PX mode indicates directory but entry '{}' is not flagged as directory",
                                name
                            )));
                        }
                    }
                    hadris_iso::susp::SystemUseField::ChildLink(cl) => {
                        if cl.child_directory_location.read() >= volume_sectors {
                            issues.push(VerifyIssue::error(format!(
                                "CL location {} exceeds volume size ({} sectors)",
                                cl.child_directory_location.read(),
                                volume_sectors
                            )));
                        }
                    }
                    hadris_iso::susp::SystemUseField::ParentLink(pl) => {
                        if pl.parent_directory_location.read() >= volume_sectors {
                            issues.push(VerifyIssue::error(format!(
                                "PL location {} exceeds volume size ({} sectors)",
                                pl.parent_directory_location.read(),
                                volume_sectors
                            )));
                        }
                    }
                    hadris_iso::susp::SystemUseField::Timestamps(tf) => {
                        use hadris_iso::rrip::TfFlags;
                        let stamp_size: usize = if tf.flags.contains(TfFlags::LONG_FORM) {
                            17
                        } else {
                            7
                        };
                        let expected_count = [
                            TfFlags::CREATION,
                            TfFlags::MODIFY,
                            TfFlags::ACCESS,
                            TfFlags::ATTRIBUTES,
                            TfFlags::BACKUP,
                            TfFlags::EXPIRATION,
                            TfFlags::EFFECTIVE,
                        ]
                        .iter()
                        .filter(|f| tf.flags.contains(**f))
                        .count();
                        let expected_len = expected_count * stamp_size;
                        if tf.timestamps.len() != expected_len {
                            let name = String::from_utf8_lossy(entry.name());
                            issues.push(VerifyIssue::warning(format!(
                                "TF timestamp data length {} doesn't match expected {} for entry '{}'",
                                tf.timestamps.len(),
                                expected_len,
                                name
                            )));
                        }
                    }
                    _ => {}
                }
            }

            if !entry.is_special()
                && is_dir
                && let Ok(child_ref) = entry.as_dir_ref(iso)
            {
                walk_rrip(iso, child_ref, volume_sectors, issues, depth + 1);
            }
        }
    }

    // Detect Rock Ridge by looking for PX/NM in root's system use area
    let has_rrip = detect_rock_ridge(iso);
    if !has_rrip {
        if verbose {
            println!("  No Rock Ridge detected, skipping RRIP checks");
        }
        return issues;
    }

    if verbose {
        println!("  Checking RRIP field correctness...");
    }

    let root = iso.root_dir();
    walk_rrip(iso, root.dir_ref(), volume_sectors, &mut issues, 0);

    issues
}

/// Verify ISO image integrity
pub fn verify(args: VerifyArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let file_size = file.metadata()?.len();
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    if args.verbose {
        println!("Verifying: {}", args.input.display());
    }

    let mut all_issues = Vec::new();

    // 1. Volume descriptors (always)
    let (vd_issues, found_pvd, boot_catalog_sector) = check_volume_descriptors(&iso, args.verbose);
    all_issues.extend(vd_issues);

    // 2. Root directory (always)
    all_issues.extend(check_root_directory(&iso, args.verbose));

    // 3. Volume size (always, if PVD was found)
    if found_pvd {
        all_issues.extend(check_volume_size(&iso, file_size, args.verbose));
    }

    // 4. Boot catalog (always, if boot record present)
    if let Some(catalog_sector) = boot_catalog_sector {
        all_issues.extend(check_boot_catalog(&iso, catalog_sector, args.verbose));
    }

    // 5. Path table consistency (strict only)
    if args.strict && found_pvd {
        all_issues.extend(check_path_table_consistency(&iso, args.verbose));
    }

    // 6. Extent bounds (strict only)
    if args.strict && found_pvd {
        all_issues.extend(check_extent_bounds(&iso, file_size, args.verbose));
    }

    // 7. RRIP fields (strict only, if Rock Ridge detected)
    if args.strict && found_pvd {
        all_issues.extend(check_rrip_fields(&iso, args.verbose));
    }

    // Report results
    let errors: Vec<_> = all_issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Error)
        .collect();
    let warnings: Vec<_> = all_issues
        .iter()
        .filter(|i| i.severity == IssueSeverity::Warning)
        .collect();

    println!();
    if errors.is_empty() && warnings.is_empty() {
        println!("Verification passed: No issues found");
        Ok(())
    } else {
        if !errors.is_empty() {
            println!("Errors ({}):", errors.len());
            for issue in &errors {
                println!("  ERROR: {}", issue.message);
            }
        }
        if !warnings.is_empty() {
            println!("Warnings ({}):", warnings.len());
            for issue in &warnings {
                println!("  WARNING: {}", issue.message);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!("{} error(s) found", errors.len()).into())
        }
    }
}

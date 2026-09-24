use std::fs::File;

use hadris_fs::FileType;
use hadris_iso::Namespace;
use hadris_iso::raw::{PathTableHeader, VolumeDescriptor};
use hadris_iso::sync::IsoImage;
use hadris_storage::host::FileDevice;

use super::super::args::VerifyArgs;

use super::{Entry, Result, View, join, list_dir, open};

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

/// Directory nesting past which the image is assumed to loop.
const MAX_DEPTH: usize = 256;

/// What the descriptor scan found.
struct Descriptors {
    found_terminator: bool,
    boot_catalog: Option<u32>,
    path_table: Option<(u32, u32)>,
}

fn check_volume_descriptors(
    iso: &mut IsoImage<FileDevice>,
    verbose: bool,
    issues: &mut Vec<VerifyIssue>,
) -> Descriptors {
    let mut found = Descriptors {
        found_terminator: false,
        boot_catalog: None,
        path_table: None,
    };
    let mut index = 0;
    loop {
        let descriptor = match iso.descriptor(index) {
            Ok(Some(descriptor)) => descriptor,
            Ok(None) => break,
            Err(error) => {
                issues.push(VerifyIssue::error(format!(
                    "Error reading volume descriptor {index}: {error}"
                )));
                break;
            }
        };
        index += 1;
        match descriptor {
            VolumeDescriptor::Primary(pvd) => {
                found.path_table = Some((pvd.type_l_path_table.get(), pvd.path_table_size.get()));
                if verbose {
                    println!("  Found Primary Volume Descriptor");
                }
            }
            VolumeDescriptor::Terminator(_) => {
                found.found_terminator = true;
                if verbose {
                    println!("  Found Volume Descriptor Set Terminator");
                }
            }
            VolumeDescriptor::BootRecord(boot) => {
                if boot.is_el_torito() {
                    found.boot_catalog = Some(boot.catalog_ptr.get());
                }
                if verbose {
                    println!(
                        "  Found Boot Record (catalog sector: {})",
                        boot.catalog_ptr.get()
                    );
                }
            }
            VolumeDescriptor::Supplementary(_) => {
                if verbose {
                    println!("  Found Supplementary Volume Descriptor");
                }
            }
            other => {
                if verbose {
                    println!(
                        "  Found Unknown Volume Descriptor (type {:?})",
                        other.header().kind()
                    );
                }
            }
        }
    }

    if !found.found_terminator {
        issues.push(VerifyIssue::error(
            "Missing Volume Descriptor Set Terminator",
        ));
    }
    found
}

fn check_volume_size(
    iso: &IsoImage<FileDevice>,
    file_size: u64,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let declared_size = u64::from(iso.volume_blocks()) * u64::from(iso.block_size());

    if verbose {
        println!("  Volume size: {declared_size} bytes (declared), {file_size} bytes (file)");
    }

    if declared_size > file_size {
        issues.push(VerifyIssue::error(format!(
            "Volume declares {declared_size} bytes but file is only {file_size} bytes (truncated image)"
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

fn check_boot_catalog(iso: &mut IsoImage<FileDevice>, verbose: bool) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let catalog = match iso.boot_catalog() {
        Ok(Some(catalog)) => catalog,
        Ok(None) => return issues,
        Err(error) => {
            issues.push(VerifyIssue::error(format!(
                "Failed to read boot catalog: {error}"
            )));
            return issues;
        }
    };

    let validation = catalog.validation();
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
    let stored = validation.checksum.get();
    let calculated = validation.expected_checksum();
    if stored != calculated {
        issues.push(VerifyIssue::error(format!(
            "Boot catalog checksum mismatch: stored {stored:#06x}, calculated {calculated:#06x}"
        )));
    }

    let volume_blocks = iso.volume_blocks();
    for entry in catalog.entries() {
        if entry.is_bootable() && entry.load_block() >= volume_blocks {
            issues.push(VerifyIssue::error(format!(
                "Boot entry image at block {} is outside the volume ({volume_blocks} blocks)",
                entry.load_block()
            )));
        }
    }

    if verbose && issues.is_empty() {
        println!(
            "  Boot catalog validation passed ({} entries)",
            catalog.entries().len()
        );
    }

    issues
}

fn check_path_table(
    iso: &mut IsoImage<FileDevice>,
    (block, size): (u32, u32),
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    let mut table = vec![0u8; size as usize];
    let offset = u64::from(block) * u64::from(iso.block_size());
    if let Err(error) = iso.read_bytes(offset, &mut table) {
        issues.push(VerifyIssue::error(format!(
            "Failed to read path table: {error}"
        )));
        return issues;
    }

    let header_len = size_of::<PathTableHeader>();
    let mut records = Vec::new();
    let mut pos = 0;
    while pos + header_len <= table.len() {
        let header: PathTableHeader = bytemuck::pod_read_unaligned(&table[pos..pos + header_len]);
        if header.len == 0 {
            break;
        }
        records.push((header.extent_le(), usize::from(header.parent_le())));
        pos += header.record_len();
    }
    if pos > table.len() {
        issues.push(VerifyIssue::error(
            "Path table record runs past the table's end",
        ));
    }

    let total = records.len();
    if verbose {
        println!("  Path table entries: {total}");
    }
    if total == 0 {
        issues.push(VerifyIssue::error("Path table is empty (no root entry)"));
        return issues;
    }
    if records[0].1 != 1 {
        issues.push(VerifyIssue::error(format!(
            "Root path table entry has parent_index {} (expected 1)",
            records[0].1
        )));
    }

    let volume_blocks = iso.volume_blocks();
    for (i, &(extent, parent)) in records.iter().enumerate() {
        let idx = i + 1;
        if parent < 1 || parent > total {
            issues.push(VerifyIssue::error(format!(
                "Path table entry {idx} has invalid parent_index {parent} (valid range: 1..{total})"
            )));
        } else if parent > idx {
            issues.push(VerifyIssue::warning(format!(
                "Path table entry {idx} names parent {parent}, which comes after it"
            )));
        }
        if extent >= volume_blocks {
            issues.push(VerifyIssue::error(format!(
                "Path table entry {idx} (LBA {extent}) is outside the volume ({volume_blocks} blocks)"
            )));
        }
    }

    issues
}

/// Every entry below the root with its path, depth first.
fn walk(view: &mut View<'_>, issues: &mut Vec<VerifyIssue>) -> Vec<(String, Entry)> {
    fn visit(
        view: &mut View<'_>,
        path: &str,
        depth: usize,
        out: &mut Vec<(String, Entry)>,
        issues: &mut Vec<VerifyIssue>,
    ) {
        if depth > MAX_DEPTH {
            issues.push(VerifyIssue::error(format!(
                "Directory nesting exceeds {MAX_DEPTH} levels (possible loop in image)"
            )));
            return;
        }
        let entries = match list_dir(view, path) {
            Ok(entries) => entries,
            Err(error) => {
                issues.push(VerifyIssue::error(format!(
                    "Error reading directory {path}: {error}"
                )));
                return;
            }
        };
        for entry in entries {
            let child = join(path, &entry.name);
            let is_dir = entry.meta.file_type().is_dir();
            out.push((child.clone(), entry));
            if is_dir {
                visit(view, &child, depth + 1, out, issues);
            }
        }
    }

    let mut out = Vec::new();
    visit(view, "/", 0, &mut out, issues);
    out
}

fn check_root_directory(view: &mut View<'_>, verbose: bool) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    match list_dir(view, "/") {
        Ok(entries) => {
            if verbose {
                let dirs = entries
                    .iter()
                    .filter(|entry| entry.meta.file_type().is_dir())
                    .count();
                println!("  Files in root: {}", entries.len() - dirs);
                println!("  Directories in root: {dirs}");
            }
        }
        Err(error) => issues.push(VerifyIssue::error(format!(
            "Error reading root directory: {error}"
        ))),
    }
    issues
}

fn check_extent_bounds(
    view: &mut View<'_>,
    entries: &[(String, Entry)],
    volume_size: u64,
    file_size: u64,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    if verbose {
        println!("  Checking extent bounds...");
    }
    for (path, entry) in entries {
        let mut extents = Vec::new();
        if let Err(error) = view.extents(entry.node, |extent| extents.push(extent)) {
            issues.push(VerifyIssue::error(format!(
                "Failed to read the extents of '{path}': {error}"
            )));
            continue;
        }
        for extent in extents.into_iter().filter(|extent| !extent.is_empty()) {
            let end = extent.end();
            if end > volume_size {
                issues.push(VerifyIssue::error(format!(
                    "Entry '{path}' extent end ({end}) exceeds volume size ({volume_size})"
                )));
            }
            if end > file_size {
                issues.push(VerifyIssue::error(format!(
                    "Entry '{path}' extent end ({end}) exceeds file size ({file_size})"
                )));
            }
        }
    }
    issues
}

fn check_rrip_fields(
    view: &mut View<'_>,
    entries: &[(String, Entry)],
    volume_blocks: u32,
    verbose: bool,
) -> Vec<VerifyIssue> {
    let mut issues = Vec::new();
    if verbose {
        println!("  Checking RRIP field correctness...");
    }
    for (path, entry) in entries {
        let info = match view.rock_ridge(entry.node) {
            Ok(Some(info)) => info,
            Ok(None) => continue,
            Err(error) => {
                issues.push(VerifyIssue::error(format!(
                    "Failed to read Rock Ridge entries of '{path}': {error}"
                )));
                continue;
            }
        };
        let is_dir = match view.raw_record(entry.node) {
            Ok(record) => record.header().is_directory(),
            Err(_) => entry.meta.file_type() == FileType::Dir,
        };
        if let Some(mode) = info.mode() {
            let file_type = mode & 0o170000;
            if is_dir && file_type != 0o040000 && file_type != 0 {
                issues.push(VerifyIssue::warning(format!(
                    "PX mode type {file_type:#o} doesn't match directory flag for '{path}'"
                )));
            } else if !is_dir && file_type == 0o040000 {
                issues.push(VerifyIssue::warning(format!(
                    "PX mode indicates directory but entry '{path}' is not flagged as directory"
                )));
            }
        }
        if let Some(block) = info.child_link()
            && block >= volume_blocks
        {
            issues.push(VerifyIssue::error(format!(
                "CL location {block} of '{path}' exceeds volume size ({volume_blocks} sectors)"
            )));
        }
        if let Some(block) = info.parent_link()
            && block >= volume_blocks
        {
            issues.push(VerifyIssue::error(format!(
                "PL location {block} of '{path}' exceeds volume size ({volume_blocks} sectors)"
            )));
        }
    }
    issues
}

/// Verify ISO image integrity
pub fn verify(args: VerifyArgs) -> Result<()> {
    let file_size = hadris_storage::host::file_len(&File::open(&args.input)?)?;
    let mut iso = open(&args.input)?;

    if args.verbose {
        println!("Verifying: {}", args.input.display());
    }

    let mut all_issues = Vec::new();
    let descriptors = check_volume_descriptors(&mut iso, args.verbose, &mut all_issues);
    all_issues.extend(check_volume_size(&iso, file_size, args.verbose));
    if descriptors.boot_catalog.is_some() {
        all_issues.extend(check_boot_catalog(&mut iso, args.verbose));
    }
    if args.strict
        && let Some(table) = descriptors.path_table
    {
        all_issues.extend(check_path_table(&mut iso, table, args.verbose));
    }

    let volume_blocks = iso.volume_blocks();
    let volume_size = u64::from(volume_blocks) * u64::from(iso.block_size());
    let has_rrip = iso.namespaces().contains(Namespace::RockRidge);
    {
        let mut view = iso.view(Namespace::Primary)?;
        all_issues.extend(check_root_directory(&mut view, args.verbose));
        if args.strict {
            let entries = walk(&mut view, &mut all_issues);
            all_issues.extend(check_extent_bounds(
                &mut view,
                &entries,
                volume_size,
                file_size,
                args.verbose,
            ));
        }
    }
    if args.strict {
        if has_rrip {
            let mut view = iso.view(Namespace::RockRidge)?;
            let entries = walk(&mut view, &mut all_issues);
            all_issues.extend(check_rrip_fields(
                &mut view,
                &entries,
                volume_blocks,
                args.verbose,
            ));
        } else if args.verbose {
            println!("  No Rock Ridge detected, skipping RRIP checks");
        }
    }

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

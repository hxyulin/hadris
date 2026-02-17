//! Size estimation for ISO 9660 images.
//!
//! Provides a way to estimate the output size of an ISO image before actually
//! writing it, which is useful for pre-allocating buffers or reporting progress.

use alloc::vec::Vec;

use crate::file::EntryType;
use crate::write::options::{CreationFeatures, FormatOptions};
use crate::write::{File, InputFiles};

/// Breakdown of the estimated size by component.
#[derive(Debug, Clone, Default)]
pub struct SizeBreakdown {
    /// System area: 16 sectors (32 KiB for 2048-byte sectors).
    pub system_area: u64,
    /// Volume descriptors (PVD, SVD, Boot Record VD, Terminator).
    pub volume_descriptors: u64,
    /// Path tables (4 per entry type: L+M for primary, L+M for supplementary).
    pub path_tables: u64,
    /// Directory records (dot, dotdot, file entries, subdirectory entries).
    pub directory_records: u64,
    /// Continuation areas for RRIP overflow.
    pub continuation_areas: u64,
    /// File data (each file rounded up to sector boundary).
    pub file_data: u64,
    /// Boot catalog (1 sector if El-Torito enabled).
    pub boot_catalog: u64,
}

/// Estimated size of an ISO image.
#[derive(Debug, Clone)]
pub struct IsoSizeEstimate {
    /// Minimum number of sectors required.
    pub minimum_sectors: u64,
    /// Breakdown of estimated size by component.
    pub breakdown: SizeBreakdown,
}

impl IsoSizeEstimate {
    /// Get the estimated minimum size in bytes.
    pub fn minimum_bytes(&self) -> u64 {
        self.minimum_sectors * 2048
    }
}

/// Count directories and files in an InputFiles tree.
struct TreeStats {
    dir_count: u64,
    file_count: u64,
    total_file_bytes: u64,
    /// Sum of converted name lengths for directories (for path table estimation).
    dir_name_bytes: u64,
    /// Sum of directory record sizes across all directories.
    dir_record_bytes: u64,
}

fn align_to_sector(bytes: u64, sector_size: u64) -> u64 {
    bytes.div_ceil(sector_size)
}

fn walk_files_stats(
    files: &[File],
    entry_types: &[EntryType],
    sector_size: u64,
    stats: &mut TreeStats,
) {
    // Each directory has dot and dotdot entries (33 bytes each + RRIP overhead)
    // plus entries for each file/subdirectory.
    let has_rrip = entry_types.iter().any(|e| e.supports_rrip());

    // Base directory overhead: dot (34 bytes) + dotdot (34 bytes) with possible RRIP
    let dot_dotdot_size: u64 = if has_rrip {
        // dot: 33 + 1(name) + ~200 (RRIP with SP+PX+NM+TF+ER for root, less for others)
        // Conservatively estimate 256 for dot and 128 for dotdot
        256 + 128
    } else {
        34 + 34
    };

    let mut dir_total = dot_dotdot_size;

    for file in files {
        match file {
            File::File { name, contents } => {
                stats.file_count += 1;
                if !contents.is_empty() {
                    stats.total_file_bytes +=
                        align_to_sector(contents.len() as u64, sector_size) * sector_size;
                }
                // Directory record for a file: 33 + name_len (padded to even)
                // + RRIP SU area if applicable
                let name_len = estimate_converted_name_len(name, entry_types);
                let record_size = if has_rrip {
                    // PX(44) + NM(5+name) + TF(19) = 68 + original_name_len
                    let su_size = 68 + name.len() as u64;
                    let base = 33 + name_len;
                    let padded = (base + 1) & !1;
                    // If SU fits inline, add to record; otherwise record is 256 max
                    (padded + su_size).min(256)
                } else {
                    let base = 33 + name_len;
                    (base + 1) & !1
                };
                dir_total += record_size;
            }
            File::Directory { name, children } => {
                stats.dir_count += 1;
                let name_len = estimate_converted_name_len(name, entry_types);
                stats.dir_name_bytes += name_len;
                // Directory record for a subdirectory entry
                let record_size = if has_rrip {
                    let su_size = 68 + name.len() as u64;
                    let base = 33 + name_len;
                    let padded = (base + 1) & !1;
                    (padded + su_size).min(256)
                } else {
                    let base = 33 + name_len;
                    (base + 1) & !1
                };
                dir_total += record_size;
                // Recurse into subdirectory
                walk_files_stats(children, entry_types, sector_size, stats);
            }
        }
    }

    // This directory's records aligned to sector
    stats.dir_record_bytes += align_to_sector(dir_total, sector_size) * sector_size;
}

/// Estimate the converted name length for the first entry type.
fn estimate_converted_name_len(name: &str, entry_types: &[EntryType]) -> u64 {
    match entry_types.first() {
        Some(EntryType::Level1 { .. }) => {
            // 8.3 format + ";1" = max 14
            let has_dot = name.contains('.');
            if has_dot {
                let dot_pos = name.find('.').unwrap();
                let basename = dot_pos.min(8);
                let ext = (name.len() - dot_pos - 1).min(3);
                (basename + 1 + ext + 2) as u64 // basename + dot + ext + ";1"
            } else {
                (name.len().min(8) + 2) as u64 // name + ";1"
            }
        }
        Some(EntryType::Level2 { .. }) => {
            // Up to 30 chars + ";1"
            (name.len().min(30) + 2) as u64
        }
        Some(EntryType::Level3 { .. }) => {
            // Up to 207 chars, no version suffix
            name.len().min(207) as u64
        }
        Some(EntryType::Joliet { .. }) => {
            // UTF-16 BE: each char is 2 bytes, max 103 code units = 206 bytes
            let code_units: usize = name.encode_utf16().count();
            (code_units.min(103) * 2) as u64
        }
        None => name.len() as u64,
    }
}

/// Estimate the output size of an ISO image before writing.
///
/// This walks the file tree once, accumulating sizes for each component.
/// The estimate is conservative (may slightly overestimate) but will never
/// underestimate the required size.
pub fn estimate(files: &InputFiles, options: &FormatOptions) -> IsoSizeEstimate {
    let sector_size = options.sector_size as u64;
    let features = &options.features;

    let entry_types = build_entry_types(features);
    let num_entry_types = entry_types.len() as u64;

    let mut breakdown = SizeBreakdown {
        system_area: 16 * sector_size,
        ..SizeBreakdown::default()
    };

    // 2. Volume descriptors
    let mut vd_count: u64 = 1; // PVD always
    if features.el_torito.is_some() {
        vd_count += 1; // Boot Record VD
    }
    if features.long_filenames {
        vd_count += 1; // SVD for L3
    }
    if features.joliet.is_some() {
        vd_count += 1; // SVD for Joliet
    }
    vd_count += 1; // Terminator
    breakdown.volume_descriptors = vd_count * sector_size;

    // 3. Walk the file tree for stats
    let mut stats = TreeStats {
        dir_count: 0,
        file_count: 0,
        total_file_bytes: 0,
        dir_name_bytes: 0,
        dir_record_bytes: 0,
    };
    walk_files_stats(&files.files, &entry_types, sector_size, &mut stats);
    // Add root directory itself
    stats.dir_count += 1;

    // 4. Path tables: one L and one M per entry type
    // Root entry: 10 bytes (8 header + 1 name + 1 padding)
    // Each subdir entry: 8 header + name_len (+ 1 padding if odd)
    let pt_root_size = 10u64;
    let pt_dir_size = stats.dir_count.saturating_sub(1).saturating_mul(8)
        + stats.dir_name_bytes
        + stats.dir_count.saturating_sub(1); // worst case padding
    let pt_size_bytes = pt_root_size + pt_dir_size;
    let pt_sectors = align_to_sector(pt_size_bytes, sector_size);
    // 2 path tables (L+M) per entry type
    breakdown.path_tables = pt_sectors * 2 * num_entry_types * sector_size;

    // 5. Directory records (already sector-aligned in stats)
    // Multiply by number of entry types since each type gets its own directory tree
    breakdown.directory_records = stats.dir_record_bytes * num_entry_types;

    // 6. Continuation areas for RRIP
    // Conservatively estimate: root dot entry often needs CE (ER entry is ~250 bytes)
    let has_rrip = entry_types.iter().any(|e| e.supports_rrip());
    if has_rrip {
        // ER entry alone is about 250 bytes; SP+PX+NM+TF+ER easily exceeds inline space.
        // Estimate one sector per directory that has RRIP for potential CE overflow.
        breakdown.continuation_areas = stats.dir_count * sector_size;
    }

    // 7. File data
    breakdown.file_data = stats.total_file_bytes;

    // 8. Boot catalog
    if features.el_torito.is_some() {
        breakdown.boot_catalog = sector_size;
    }

    let total_bytes = breakdown.system_area
        + breakdown.volume_descriptors
        + breakdown.path_tables
        + breakdown.directory_records
        + breakdown.continuation_areas
        + breakdown.file_data
        + breakdown.boot_catalog;

    let minimum_sectors = align_to_sector(total_bytes, sector_size);

    IsoSizeEstimate {
        minimum_sectors,
        breakdown,
    }
}

fn build_entry_types(features: &CreationFeatures) -> Vec<EntryType> {
    let mut entry_types = Vec::new();
    entry_types.push(features.filenames.into());
    if features.long_filenames {
        entry_types.push(EntryType::Level3 {
            supports_lowercase: true,
            supports_rrip: false,
        });
    }
    if let Some(joliet) = features.joliet {
        entry_types.push(joliet.into());
    }
    entry_types
}

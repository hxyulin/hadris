//! Command implementations for hadris-iso CLI

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, Write};
use std::num::NonZeroU16;

use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::boot::{EmulationType, PlatformId};
use hadris_iso::directory::FileFlags;
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::types::Endian;
use hadris_iso::volume::VolumeDescriptor;
use hadris_iso::write::options::{CreationFeatures, FormatOptions, HybridBootOptions};
use hadris_iso::write::{InputFiles, IsoImageWriter};

use crate::args::{
    CreateArgs, ExtractArgs, InfoArgs, LsArgs, MkisofsArgs, TreeArgs, VerifyArgs,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Display information about an ISO image
pub fn info(args: InfoArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    println!("ISO 9660 Image: {}", args.input.display());
    println!();

    // Read and display volume descriptors
    let mut has_boot = false;
    let mut has_joliet = false;
    let has_rockridge = false; // TODO: detect Rock Ridge from SUSP entries

    for vd in iso.read_volume_descriptors() {
        let vd = vd?;
        match vd {
            VolumeDescriptor::Primary(pvd) => {
                println!("Primary Volume Descriptor:");
                println!("  Volume ID:        {}", pvd.volume_identifier);
                println!("  System ID:        {}", pvd.system_identifier);
                println!("  Volume Set ID:    {}", pvd.volume_set_identifier);
                println!("  Publisher ID:     {}", pvd.publisher_identifier);
                println!("  Preparer ID:      {}", pvd.preparer_identifier);
                println!("  Application ID:   {}", pvd.application_identifier);
                println!("  Volume Size:      {} sectors ({} bytes)",
                    pvd.volume_space_size.read(),
                    pvd.volume_space_size.read() as u64 * 2048);
                println!("  Block Size:       {} bytes", pvd.logical_block_size.read());
                println!("  Path Table Size:  {} bytes", pvd.path_table_size.read());
                if args.verbose {
                    println!("  Root Extent:      sector {}", pvd.dir_record.header.extent.read());
                    println!("  Root Size:        {} bytes", pvd.dir_record.header.data_len.read());
                }
            }
            VolumeDescriptor::BootRecord(boot) => {
                has_boot = true;
                println!();
                println!("Boot Record (El-Torito):");
                let sys_id = core::str::from_utf8(&boot.boot_system_identifier)
                    .unwrap_or("<invalid>")
                    .trim();
                println!("  System ID:        {}", sys_id);
                println!("  Catalog Sector:   {}", boot.catalog_ptr.get());
            }
            VolumeDescriptor::Supplementary(svd) => {
                // Check for Joliet
                for level in JolietLevel::all() {
                    if svd.escape_sequences == level.escape_sequence() {
                        has_joliet = true;
                        println!();
                        println!("Joliet Extension ({:?}):", level);
                        println!("  Volume ID:        {}", svd.volume_identifier);
                        break;
                    }
                }
                // Check for enhanced volume descriptor
                if svd.file_structure_version == 2 {
                    println!();
                    println!("Enhanced Volume Descriptor (ISO 9660:1999):");
                    println!("  Volume ID:        {}", svd.volume_identifier);
                }
            }
            VolumeDescriptor::End(_) => {}
            VolumeDescriptor::Unknown(_) => {
                if args.verbose {
                    println!();
                    println!("Unknown Volume Descriptor");
                }
            }
        }
    }

    // Summary
    println!();
    println!("Features:");
    println!("  El-Torito Boot:   {}", if has_boot { "Yes" } else { "No" });
    println!("  Joliet:           {}", if has_joliet { "Yes" } else { "No" });
    println!("  Rock Ridge:       {}", if has_rockridge { "Yes" } else { "No" });

    Ok(())
}

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    let root = iso.root_dir();

    for entry in root.iter(&iso).entries() {
        let entry = entry?;
        let name_bytes = entry.name();
        let name = String::from_utf8_lossy(name_bytes);

        // Handle special entries
        let display_name = match name_bytes {
            [0x00] => {
                if !args.all {
                    continue;
                }
                ".".to_string()
            }
            [0x01] => {
                if !args.all {
                    continue;
                }
                "..".to_string()
            }
            _ => {
                // Strip version number (;1) if present
                let name_str = name.to_string();
                if let Some(pos) = name_str.rfind(';') {
                    name_str[..pos].to_string()
                } else {
                    name_str
                }
            }
        };

        let flags = FileFlags::from_bits_truncate(entry.header().flags);

        if args.long {
            let type_char = if flags.contains(FileFlags::DIRECTORY) {
                'd'
            } else {
                '-'
            };
            let size = entry.header().data_len.read();
            let extent = entry.header().extent.read();

            println!(
                "{}  {:>10}  {:>8}  {}",
                type_char,
                size,
                extent,
                display_name
            );
        } else {
            if flags.contains(FileFlags::DIRECTORY) {
                println!("{}/", display_name);
            } else {
                println!("{}", display_name);
            }
        }
    }

    Ok(())
}

/// Display directory tree
pub fn tree(args: TreeArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    println!("{}", args.path);

    let root = iso.root_dir();
    let max_depth = args.depth.unwrap_or(usize::MAX);

    print_tree_recursive(&iso, &root, "", true, 0, max_depth)?;

    Ok(())
}

fn print_tree_recursive<R: Read + Seek>(
    iso: &IsoImage<R>,
    dir: &hadris_iso::read::RootDir,
    prefix: &str,
    _is_last: bool,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth >= max_depth {
        return Ok(());
    }

    let entries: Vec<_> = dir
        .iter(iso)
        .entries()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.name();
            !matches!(name, [0x00] | [0x01])
        })
        .collect();

    for (idx, entry) in entries.iter().enumerate() {
        let is_last_entry = idx == entries.len() - 1;
        let connector = if is_last_entry { "└── " } else { "├── " };
        let name = String::from_utf8_lossy(entry.name());

        // Strip version number
        let display_name = if let Some(pos) = name.rfind(';') {
            &name[..pos]
        } else {
            &name
        };

        let flags = FileFlags::from_bits_truncate(entry.header().flags);
        let suffix = if flags.contains(FileFlags::DIRECTORY) {
            "/"
        } else {
            ""
        };

        println!("{}{}{}{}", prefix, connector, display_name, suffix);

        // Note: Recursive directory traversal would require following the directory extent
        // This is a simplified implementation that only shows the root directory
    }

    Ok(())
}

/// Extract files from an ISO image
pub fn extract(args: ExtractArgs) -> Result<()> {
    // Open the ISO for reading directory structure
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    // Create output directory
    fs::create_dir_all(&args.output)?;

    // Open a second file handle for reading file contents
    let mut content_reader = File::open(&args.input)?;

    let root = iso.root_dir();
    let mut extracted_count = 0;

    for entry in root.iter(&iso).entries() {
        let entry = entry?;
        let name_bytes = entry.name();

        // Skip . and ..
        if matches!(name_bytes, [0x00] | [0x01]) {
            continue;
        }

        let name = String::from_utf8_lossy(name_bytes);
        let flags = FileFlags::from_bits_truncate(entry.header().flags);

        // Skip directories for now (would need recursive extraction)
        if flags.contains(FileFlags::DIRECTORY) {
            if args.verbose {
                println!("Skipping directory: {}", name);
            }
            continue;
        }

        // Strip version number
        let filename = if let Some(pos) = name.rfind(';') {
            &name[..pos]
        } else {
            &name
        };

        let output_path = args.output.join(filename);

        if args.verbose {
            println!("Extracting: {} ({} bytes)", filename, entry.header().data_len.read());
        }

        // Read file content from ISO
        let extent = entry.header().extent.read() as u64;
        let size = entry.header().data_len.read() as usize;

        content_reader.seek(io::SeekFrom::Start(extent * 2048))?;
        let mut buffer = vec![0u8; size];
        content_reader.read_exact(&mut buffer)?;

        // Write to output file
        let mut output_file = File::create(&output_path)?;
        output_file.write_all(&buffer)?;

        extracted_count += 1;
    }

    println!("Extracted {} files to {}", extracted_count, args.output.display());
    Ok(())
}

/// Create a new ISO image
pub fn create(args: CreateArgs) -> Result<()> {
    if args.verbose {
        println!("Creating ISO from: {}", args.source.display());
        println!("Output: {}", args.output.display());
    }

    // Gather input files
    let input = InputFiles::from_fs(&args.source, PathSeparator::ForwardSlash)?;

    if args.verbose {
        println!("Found {} files/directories", count_files(&input));
    }

    // Configure boot options
    let el_torito = if let Some(boot_path) = &args.boot {
        let mut boot_opts = BootOptions {
            write_boot_catalog: true,
            default: BootEntryOptions {
                boot_image_path: normalize_path(boot_path),
                load_size: NonZeroU16::new(args.boot_load_size),
                boot_info_table: args.boot_info_table,
                grub2_boot_info: false,
                emulation: EmulationType::NoEmulation,
            },
            entries: vec![],
        };

        // Add UEFI boot entry if specified
        if let Some(efi_path) = &args.efi_boot {
            boot_opts.entries.push((
                BootSectionOptions {
                    platform: PlatformId::UEFI,
                },
                BootEntryOptions {
                    boot_image_path: normalize_path(efi_path),
                    load_size: None,
                    boot_info_table: false,
                    grub2_boot_info: false,
                    emulation: EmulationType::NoEmulation,
                },
            ));
        }

        Some(boot_opts)
    } else {
        None
    };

    // Configure hybrid boot
    let hybrid_boot = if args.hybrid_mbr && args.hybrid_gpt {
        Some(HybridBootOptions::hybrid())
    } else if args.hybrid_gpt {
        Some(HybridBootOptions::gpt())
    } else if args.hybrid_mbr {
        Some(HybridBootOptions::mbr())
    } else {
        None
    };

    // Configure format options
    let format_options = FormatOptions {
        volume_name: args.volume_name.clone(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: args.level.0.clone(),
            long_filenames: false,
            joliet: if args.joliet {
                Some(JolietLevel::Level3)
            } else {
                None
            },
            rock_ridge: None, // TODO: implement rock ridge options
            el_torito,
            hybrid_boot,
        },
    };

    // Create output buffer with estimated size
    let estimated_size = estimate_iso_size(&input);
    let mut buffer = io::Cursor::new(vec![0u8; estimated_size as usize]);

    // Write ISO to buffer
    IsoImageWriter::format_new(&mut buffer, input, format_options)?;

    // Seek to end to get actual size used
    buffer.seek(io::SeekFrom::End(0))?;
    let mut actual_size = buffer.position() as usize;

    // ISO must be at least 16 sectors (volume descriptors start at sector 16)
    // plus some data, so minimum is around 20 sectors
    let min_size = 32 * 2048; // 32 sectors minimum
    if actual_size < min_size {
        actual_size = min_size;
    }

    let data = buffer.into_inner();

    // Write the ISO to file
    let mut file = File::create(&args.output)?;
    file.write_all(&data[..actual_size])?;

    if args.verbose {
        println!("Created ISO: {} ({} bytes)", args.output.display(), actual_size);
    } else {
        println!("Created: {}", args.output.display());
    }

    Ok(())
}

/// Verify ISO image integrity
pub fn verify(args: VerifyArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    let mut issues = Vec::new();

    if args.verbose {
        println!("Verifying: {}", args.input.display());
    }

    // Check volume descriptors
    let mut found_pvd = false;
    let mut found_terminator = false;

    for vd in iso.read_volume_descriptors() {
        match vd {
            Ok(VolumeDescriptor::Primary(_)) => {
                found_pvd = true;
                if args.verbose {
                    println!("  Found Primary Volume Descriptor");
                }
            }
            Ok(VolumeDescriptor::End(_)) => {
                found_terminator = true;
                if args.verbose {
                    println!("  Found Volume Descriptor Set Terminator");
                }
            }
            Ok(VolumeDescriptor::BootRecord(_)) => {
                if args.verbose {
                    println!("  Found Boot Record");
                }
            }
            Ok(VolumeDescriptor::Supplementary(_)) => {
                if args.verbose {
                    println!("  Found Supplementary Volume Descriptor");
                }
            }
            Ok(VolumeDescriptor::Unknown(u)) => {
                if args.verbose {
                    println!("  Found Unknown Volume Descriptor (type {:?})", u);
                }
            }
            Err(e) => {
                issues.push(format!("Error reading volume descriptor: {}", e));
            }
        }
    }

    if !found_pvd {
        issues.push("Missing Primary Volume Descriptor".to_string());
    }

    if !found_terminator {
        issues.push("Missing Volume Descriptor Set Terminator".to_string());
    }

    // Check root directory
    let root = iso.root_dir();
    let mut file_count = 0;
    let mut dir_count = 0;

    for entry in root.iter(&iso).entries() {
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
                issues.push(format!("Error reading directory entry: {}", e));
            }
        }
    }

    if args.verbose {
        println!("  Files in root: {}", file_count);
        println!("  Directories in root: {}", dir_count);
    }

    // Report results
    println!();
    if issues.is_empty() {
        println!("Verification passed: No issues found");
        Ok(())
    } else {
        println!("Verification found {} issue(s):", issues.len());
        for issue in &issues {
            println!("  - {}", issue);
        }
        Err(format!("{} verification issue(s) found", issues.len()).into())
    }
}

/// xorriso-compatible mkisofs mode
pub fn mkisofs(args: MkisofsArgs) -> Result<()> {
    let output_path = args.output.clone().unwrap_or_else(|| {
        let mut p = args.source.clone();
        p.set_extension("iso");
        p
    });

    // Gather input files
    let input = InputFiles::from_fs(&args.source, PathSeparator::ForwardSlash)?;

    // Configure boot options
    let el_torito = if let Some(boot_path) = &args.boot_image {
        Some(BootOptions {
            write_boot_catalog: true,
            default: BootEntryOptions {
                boot_image_path: normalize_path(boot_path),
                load_size: NonZeroU16::new(args.boot_load_size.unwrap_or(4)),
                boot_info_table: args.boot_info_table,
                grub2_boot_info: false,
                // Note: Only NoEmulation is currently supported
                emulation: EmulationType::NoEmulation,
            },
            entries: if let Some(efi_path) = &args.efi_boot {
                vec![(
                    BootSectionOptions {
                        platform: PlatformId::UEFI,
                    },
                    BootEntryOptions {
                        boot_image_path: normalize_path(efi_path),
                        load_size: None,
                        boot_info_table: false,
                        grub2_boot_info: false,
                        emulation: EmulationType::NoEmulation,
                    },
                )]
            } else {
                vec![]
            },
        })
    } else {
        None
    };

    // Configure hybrid boot
    let hybrid_boot = if args.isohybrid_mbr.is_some() {
        Some(HybridBootOptions::mbr())
    } else {
        None
    };

    // Configure format options
    let format_options = FormatOptions {
        volume_name: args.volume_name.unwrap_or_else(|| "CDROM".to_string()),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: hadris_iso::write::options::BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: args.rock_ridge,
            },
            long_filenames: false,
            joliet: if args.joliet {
                Some(JolietLevel::Level3)
            } else {
                None
            },
            rock_ridge: None,
            el_torito,
            hybrid_boot,
        },
    };

    // Create output buffer with estimated size
    let estimated_size = estimate_iso_size(&input);
    let mut buffer = io::Cursor::new(vec![0u8; estimated_size as usize]);

    // Write ISO to buffer
    IsoImageWriter::format_new(&mut buffer, input, format_options)?;

    // Seek to end to get actual size
    buffer.seek(io::SeekFrom::End(0))?;
    let mut actual_size = buffer.position() as usize;

    // ISO must be at least 32 sectors
    let min_size = 32 * 2048;
    if actual_size < min_size {
        actual_size = min_size;
    }

    let data = buffer.into_inner();

    // Write the ISO to file
    let mut file = File::create(&output_path)?;
    file.write_all(&data[..actual_size])?;

    println!("Written to {} ({} bytes)", output_path.display(), actual_size);

    Ok(())
}

// Helper functions

/// Normalize a path to use forward slashes (ISO 9660 standard).
/// This ensures Windows-style backslashes work correctly.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn count_files(input: &InputFiles) -> usize {
    fn count_recursive(files: &[hadris_iso::write::File]) -> usize {
        files
            .iter()
            .map(|f| match f {
                hadris_iso::write::File::File { .. } => 1,
                hadris_iso::write::File::Directory { children, .. } => 1 + count_recursive(children),
            })
            .sum()
    }
    count_recursive(&input.files)
}

fn estimate_iso_size(input: &InputFiles) -> u64 {
    fn size_recursive(files: &[hadris_iso::write::File]) -> u64 {
        files
            .iter()
            .map(|f| match f {
                hadris_iso::write::File::File { contents, .. } => {
                    // Round up to sector boundary
                    ((contents.len() as u64 + 2047) / 2048) * 2048
                }
                hadris_iso::write::File::Directory { children, .. } => {
                    2048 + size_recursive(children) // Directory entry + children
                }
            })
            .sum()
    }

    // Base overhead: 16 system sectors + volume descriptors + path tables
    let base_overhead = 32 * 2048;
    let content_size = size_recursive(&input.files);

    base_overhead + content_size + (1024 * 1024) // Add 1MB buffer
}

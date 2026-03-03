use std::fs::File;
use std::io::{self, Write};
use std::num::NonZeroU16;

use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::boot::{EmulationType, PlatformId};
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::PathSeparator;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{CreationFeatures, FormatOptions, HybridBootOptions};
use hadris_iso::write::{InputFiles, IsoImageWriter, estimator};

use crate::args::CreateArgs;

use super::{Result, compute_estimated_size, count_files, normalize_path};

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

    // Configure Rock Ridge
    let mut filenames = args.level.0;
    if args.rock_ridge {
        match &mut filenames {
            hadris_iso::write::options::BaseIsoLevel::Level1 { supports_rrip, .. } => {
                *supports_rrip = true
            }
            hadris_iso::write::options::BaseIsoLevel::Level2 { supports_rrip, .. } => {
                *supports_rrip = true
            }
        }
    }

    // Configure format options
    let format_options = FormatOptions {
        volume_name: args.volume_name.clone(),
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames,
            long_filenames: false,
            joliet: if args.joliet {
                Some(JolietLevel::Level3)
            } else {
                None
            },
            rock_ridge: if args.rock_ridge {
                Some(RripOptions::default())
            } else {
                None
            },
            el_torito,
            hybrid_boot,
        },
    };

    // Dry run: print estimate and exit
    if args.dry_run {
        let est = estimator::estimate(&input, &format_options);
        println!(
            "Estimated size: {} bytes ({} sectors)",
            est.minimum_bytes(),
            est.minimum_sectors
        );
        println!("  System area:        {:>10}", est.breakdown.system_area);
        println!(
            "  Volume descriptors: {:>10}",
            est.breakdown.volume_descriptors
        );
        println!("  Path tables:        {:>10}", est.breakdown.path_tables);
        println!(
            "  Directory records:  {:>10}",
            est.breakdown.directory_records
        );
        println!(
            "  Continuation areas: {:>10}",
            est.breakdown.continuation_areas
        );
        println!("  File data:          {:>10}", est.breakdown.file_data);
        println!("  Boot catalog:       {:>10}", est.breakdown.boot_catalog);
        return Ok(());
    }

    // Create output buffer with estimated size
    let estimated_size = compute_estimated_size(&input, &format_options);
    let mut buffer = io::Cursor::new(vec![0u8; estimated_size as usize]);

    // Write ISO to buffer
    IsoImageWriter::format_new(&mut buffer, input, format_options)?;

    // Read volume_space_size from PVD (LE u32 at byte offset 32848)
    let data = buffer.into_inner();
    let vol_size_le = &data[32848..32852];
    let volume_sectors = u32::from_le_bytes(vol_size_le.try_into().unwrap()) as usize;
    let actual_size = (volume_sectors * 2048).max(32 * 2048);

    // Write the ISO to file
    let mut file = File::create(&args.output)?;
    file.write_all(&data[..actual_size])?;

    if args.verbose {
        println!(
            "Created ISO: {} ({} bytes)",
            args.output.display(),
            actual_size
        );
    } else {
        println!("Created: {}", args.output.display());
    }

    Ok(())
}

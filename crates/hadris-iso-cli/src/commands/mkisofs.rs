use std::fs::File;
use std::io::{self, Seek, Write};
use std::num::NonZeroU16;

use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::boot::{EmulationType, PlatformId};
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::PathSeparator;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{CreationFeatures, FormatOptions, HybridBootOptions};
use hadris_iso::write::{InputFiles, IsoImageWriter};

use crate::args::MkisofsArgs;

use super::{Result, compute_estimated_size, normalize_path};

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
        path_separator: PathSeparator::ForwardSlash,
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
            rock_ridge: if args.rock_ridge {
                Some(RripOptions::default())
            } else {
                None
            },
            el_torito,
            hybrid_boot,
        },
    };

    // Create output buffer with estimated size
    let estimated_size = compute_estimated_size(&input, &format_options);
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

    println!(
        "Written to {} ({} bytes)",
        output_path.display(),
        actual_size
    );

    Ok(())
}

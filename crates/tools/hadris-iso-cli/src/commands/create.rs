use hadris_fs::SystemClock;
use hadris_iso::sync::plan;
use hadris_iso::{
    BootEntry, BootInfo, Charset, ElTorito, HybridBoot, IsoOptions, JolietLevel, Platform,
    RockRidge, VolumeIdentifiers,
};

use super::super::args::CreateArgs;

use super::{Result, normalize_path, print_warnings, read_source, write_image};

/// The path of the visible boot catalog in created images.
pub(super) const CATALOG_PATH: &str = "boot.catalog";

/// Create a new ISO image
pub fn create(args: CreateArgs) -> Result<()> {
    if args.verbose {
        println!("Creating ISO from: {}", args.source.display());
        println!("Output: {}", args.output.display());
    }

    let tree = read_source(&args.source)?;

    let mut volume = VolumeIdentifiers::new(args.volume_name.clone());
    if let Some(id) = &args.system_id {
        volume = volume.with_system(id.clone());
    }
    if let Some(id) = &args.volume_set_id {
        volume = volume.with_volume_set(id.clone());
    }
    if let Some(id) = &args.publisher_id {
        volume = volume.with_publisher(id.clone());
    }
    if let Some(id) = &args.preparer_id {
        volume = volume.with_preparer(id.clone());
    }
    if let Some(id) = &args.application_id {
        volume = volume.with_application(id.clone());
    }

    let mut options = IsoOptions::default()
        .with_volume(volume)
        .with_level(args.level.level)
        .with_name_case(args.level.name_case)
        .with_clock(SystemClock);
    if args.strict_charset {
        options = options.with_charset(Charset::Strict);
    }
    if args.joliet {
        options = options.with_joliet(JolietLevel::L3);
    }
    if args.rock_ridge {
        options = options.with_rock_ridge(RockRidge::default());
    }

    if let Some(boot_path) = &args.boot {
        let mut bios = BootEntry::new(normalize_path(boot_path));
        if args.boot_load_size != 0 {
            bios = bios.with_load_size(args.boot_load_size);
        }
        if args.boot_info_table {
            bios = bios.with_boot_info_table(BootInfo::Standard);
        }
        let mut el_torito = ElTorito::new(bios).with_catalog_path(CATALOG_PATH);
        if let Some(efi_path) = &args.efi_boot {
            el_torito = el_torito
                .with_entry(BootEntry::new(normalize_path(efi_path)).with_platform(Platform::Efi));
        }
        options = options.with_el_torito(el_torito);
    }

    let hybrid = match (args.hybrid_mbr, args.hybrid_gpt) {
        (true, true) => Some(HybridBoot::hybrid()),
        (false, true) => Some(HybridBoot::gpt()),
        (true, false) => Some(HybridBoot::mbr()),
        (false, false) => None,
    };
    if let Some(hybrid) = hybrid {
        options = options.with_hybrid(hybrid);
    }

    if args.dry_run {
        let report = plan(&tree, &options)?;
        println!(
            "Estimated size: {} bytes ({} sectors)",
            report.size_bytes(),
            report.total_blocks()
        );
        print_warnings(&report, args.verbose);
        return Ok(());
    }

    let report = write_image(&args.output, &tree, &options, args.verbose)?;

    if args.verbose {
        println!(
            "Created ISO: {} ({} bytes)",
            args.output.display(),
            report.size_bytes()
        );
    } else {
        println!("Created: {}", args.output.display());
    }

    Ok(())
}

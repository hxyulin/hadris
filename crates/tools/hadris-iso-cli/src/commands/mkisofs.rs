use hadris_fs::SystemClock;
use hadris_iso::{
    BootEntry, BootInfo, ElTorito, HybridBoot, IsoOptions, JolietLevel, Platform, RockRidge,
    VolumeIdentifiers,
};

use super::super::args::MkisofsArgs;

use super::create::CATALOG_PATH;
use super::{Result, normalize_path, read_source, write_image};

/// xorriso-compatible mkisofs mode
pub fn mkisofs(args: MkisofsArgs) -> Result<()> {
    let output_path = args.output.clone().unwrap_or_else(|| {
        let mut p = args.source.clone();
        p.set_extension("iso");
        p
    });

    let tree = read_source(&args.source)?;

    let volume = args.volume_name.as_deref().unwrap_or("CDROM");
    let mut options = IsoOptions::default()
        .with_volume(VolumeIdentifiers::new(volume))
        .with_clock(SystemClock);
    if args.joliet {
        options = options.with_joliet(JolietLevel::L3);
    }
    if args.rock_ridge {
        options = options.with_rock_ridge(RockRidge::default());
    }

    if let Some(boot_path) = &args.boot_image {
        let mut bios = BootEntry::new(normalize_path(boot_path))
            .with_load_size(args.boot_load_size.unwrap_or(4));
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

    if let Some(mbr) = &args.isohybrid_mbr {
        options = options.with_hybrid(HybridBoot::mbr().with_bootstrap(std::fs::read(mbr)?));
    }

    let report = write_image(&output_path, &tree, &options, false)?;
    println!(
        "Written to {} ({} bytes)",
        output_path.display(),
        report.size_bytes().max(32 * 2048)
    );

    Ok(())
}

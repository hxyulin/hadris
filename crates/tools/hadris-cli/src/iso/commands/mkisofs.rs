use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoId, IsoOptions};

use super::super::args::MkisofsArgs;

use super::create::CATALOG_PATH;
use super::{Result, build_time, normalize_path, read_source, write_image};

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
        .with_id(IsoId::Volume, volume)
        .with_time(build_time()?);
    if args.joliet {
        options = options.with_joliet();
    }
    if args.rock_ridge {
        options = options.with_rock_ridge();
    }

    if let Some(boot_path) = &args.boot_image {
        let mut bios = BootEntry::bios(&normalize_path(boot_path))
            .with_load_size(args.boot_load_size.unwrap_or(4));
        if args.boot_info_table {
            bios = bios.with_boot_info(BootInfo::Table);
        }
        let mut el_torito = ElTorito::new()
            .with_entry(bios)
            .with_catalog_path(CATALOG_PATH);
        if let Some(efi_path) = &args.efi_boot {
            el_torito = el_torito.with_entry(BootEntry::uefi(&normalize_path(efi_path)));
        }
        options = options.with_el_torito(el_torito);
    }

    if let Some(mbr) = &args.isohybrid_mbr {
        options = options.with_hybrid(Hybrid::mbr().with_bootstrap(&std::fs::read(mbr)?));
    }

    let target = crate::common::Target {
        output: output_path.clone(),
        force: args.force,
    };
    let report = write_image(&target, &tree, &options, false)?;
    println!(
        "Written to {} ({} bytes)",
        output_path.display(),
        report.size().max(32 * 2048)
    );

    Ok(())
}

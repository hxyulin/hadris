use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoId, IsoOptions};

use super::super::args::CreateArgs;

use super::{Result, build_time, normalize_path, print_warnings, read_source, write_image};

/// The path of the visible boot catalog in created images.
pub(super) const CATALOG_PATH: &str = "boot.catalog";

/// Create a new ISO image
pub fn create(args: CreateArgs) -> Result<()> {
    if args.verbose {
        println!("Creating ISO from: {}", args.source.display());
        println!("Output: {}", args.output.display());
    }

    let tree = read_source(&args.source)?;

    let mut options = IsoOptions::default()
        .with_level(args.level.level)
        .with_name_case(args.level.name_case)
        .with_time(build_time()?);
    let ids = [
        (IsoId::Volume, Some(&args.volume_name), true),
        (IsoId::System, args.system_id.as_ref(), false),
        (IsoId::VolumeSet, args.volume_set_id.as_ref(), true),
        (IsoId::Publisher, args.publisher_id.as_ref(), false),
        (IsoId::Preparer, args.preparer_id.as_ref(), false),
        (IsoId::Application, args.application_id.as_ref(), false),
    ];
    for (id, value, d_chars) in ids {
        if let Some(value) = value {
            let value = if args.strict_charset {
                strict(value, d_chars)
            } else {
                value.clone()
            };
            options = options.with_id(id, &value);
        }
    }
    if args.joliet {
        options = options.with_joliet();
    }
    if args.rock_ridge {
        options = options.with_rock_ridge();
    }

    if let Some(boot_path) = &args.boot {
        let mut bios = BootEntry::bios(&normalize_path(boot_path));
        if args.boot_load_size != 0 {
            bios = bios.with_load_size(args.boot_load_size);
        }
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

    let hybrid = match (args.hybrid_mbr, args.hybrid_gpt) {
        (true, true) => Some(Hybrid::gpt_hybrid_mbr()),
        (false, true) => Some(Hybrid::gpt()),
        (true, false) => Some(Hybrid::mbr()),
        (false, false) => None,
    };
    if let Some(hybrid) = hybrid {
        options = options.with_hybrid(hybrid);
    }

    if args.dry_run {
        let report = hadris_iso::plan(&tree, &options)?;
        println!(
            "Estimated size: {} bytes ({} sectors)",
            report.size(),
            report.size() / 2048
        );
        print_warnings(&report, args.verbose);
        return Ok(());
    }

    let report = write_image(&args.output, &tree, &options, args.verbose)?;

    if args.verbose {
        println!(
            "Created ISO: {} ({} bytes)",
            args.output.display(),
            report.size()
        );
    } else {
        println!("Created: {}", args.output.display());
    }

    Ok(())
}

/// `id` in the ECMA-119 d-characters, or a-characters without `d_chars`:
/// lowercase becomes uppercase and other characters `_`.
fn strict(id: &str, d_chars: bool) -> String {
    id.chars()
        .map(|ch| {
            let ch = ch.to_ascii_uppercase();
            if ch.is_ascii_uppercase()
                || ch.is_ascii_digit()
                || ch == '_'
                || (!d_chars && " !\"%&'()*+,-./:;<=>?".contains(ch))
            {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

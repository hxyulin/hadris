use hadris_fs::DateTime;
use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoId, IsoOptions};

use super::super::args::{CreateArgs, IsoFlags};

use super::{Result, build_time, normalize_path, print_warnings, read_source, write_image};

/// The path of the visible boot catalog in created images.
pub(super) const CATALOG_PATH: &str = "boot.catalog";

/// Create a new ISO image
pub fn create(args: CreateArgs) -> Result<()> {
    let output = &args.target.output;
    if args.verbose {
        println!("Creating ISO from: {}", args.source.display());
        println!("Output: {}", output.display());
    }

    let tree = read_source(&args.source)?;
    let options = iso_options(&args.iso, build_time()?);

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

    let report = write_image(&args.target, &tree, &options, args.verbose)?;

    if args.verbose {
        println!(
            "Created ISO: {} ({} bytes)",
            output.display(),
            report.size()
        );
    } else {
        println!("Created: {}", output.display());
    }

    Ok(())
}

/// The writer options `flags` ask for, dated `time`.
pub fn iso_options(flags: &IsoFlags, time: DateTime) -> IsoOptions {
    let mut options = IsoOptions::default()
        .with_level(flags.level.level)
        .with_name_case(flags.level.name_case)
        .with_time(time);
    let ids = [
        (IsoId::Volume, Some(&flags.volume_name), true),
        (IsoId::System, flags.system_id.as_ref(), false),
        (IsoId::VolumeSet, flags.volume_set_id.as_ref(), true),
        (IsoId::Publisher, flags.publisher_id.as_ref(), false),
        (IsoId::Preparer, flags.preparer_id.as_ref(), false),
        (IsoId::Application, flags.application_id.as_ref(), false),
    ];
    for (id, value, d_chars) in ids {
        if let Some(value) = value {
            let value = if flags.strict_charset {
                strict(value, d_chars)
            } else {
                value.clone()
            };
            options = options.with_id(id, &value);
        }
    }
    if flags.joliet {
        options = options.with_joliet();
    }
    if flags.rock_ridge {
        options = options.with_rock_ridge();
    }

    let mut entries = Vec::new();
    if let Some(boot_path) = &flags.boot {
        let mut bios = BootEntry::bios(&normalize_path(boot_path));
        if flags.boot_load_size != 0 {
            bios = bios.with_load_size(flags.boot_load_size);
        }
        if flags.boot_info_table {
            bios = bios.with_boot_info(BootInfo::Table);
        }
        entries.push(bios);
    }
    if let Some(efi_path) = &flags.efi_boot {
        entries.push(BootEntry::uefi(&normalize_path(efi_path)));
    }
    if !entries.is_empty() {
        let mut el_torito = ElTorito::new().with_catalog_path(CATALOG_PATH);
        for entry in entries {
            el_torito = el_torito.with_entry(entry);
        }
        options = options.with_el_torito(el_torito);
    }

    let hybrid = match (flags.hybrid_mbr, flags.hybrid_gpt) {
        (true, true) => Some(Hybrid::gpt_hybrid_mbr()),
        (false, true) => Some(Hybrid::gpt()),
        (true, false) => Some(Hybrid::mbr()),
        (false, false) => None,
    };
    if let Some(hybrid) = hybrid {
        options = options.with_hybrid(hybrid);
    }
    options
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

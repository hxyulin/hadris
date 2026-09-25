use hadris_iso::raw::VolumeDescriptor;
use hadris_iso::{JolietLevel, Namespace};

use super::super::args::InfoArgs;

use super::{Result, open};

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ucs2(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .trim_end_matches([' ', '\0'])
        .to_string()
}

/// Display information about an ISO image
pub fn info(args: InfoArgs) -> Result<()> {
    let mut dev = open(&args.input)?;
    let mut iso = super::view(&mut dev, hadris_iso::Namespace::Preferred)?;
    let block_size = u64::from(iso.info().block_size());

    println!("ISO 9660 Image: {}", args.input.display());
    println!();

    let mut index = 0;
    while let Some(descriptor) = iso.descriptor(index)? {
        index += 1;
        match descriptor {
            VolumeDescriptor::Primary(pvd) => {
                println!("Primary Volume Descriptor:");
                println!(
                    "  Volume ID:        {}",
                    text(pvd.volume_identifier.trimmed())
                );
                println!(
                    "  System ID:        {}",
                    text(pvd.system_identifier.trimmed())
                );
                println!(
                    "  Volume Set ID:    {}",
                    text(pvd.volume_set_identifier.trimmed())
                );
                println!(
                    "  Publisher ID:     {}",
                    text(pvd.publisher_identifier.trimmed())
                );
                println!(
                    "  Preparer ID:      {}",
                    text(pvd.preparer_identifier.trimmed())
                );
                println!(
                    "  Application ID:   {}",
                    text(pvd.application_identifier.trimmed())
                );
                let blocks = u64::from(pvd.volume_space_size.get());
                println!(
                    "  Volume Size:      {blocks} sectors ({} bytes)",
                    blocks * block_size
                );
                println!("  Block Size:       {block_size} bytes");
                println!("  Path Table Size:  {} bytes", pvd.path_table_size.get());
                if args.verbose {
                    println!(
                        "  Root Extent:      sector {}",
                        pvd.root.header.extent.get()
                    );
                    println!(
                        "  Root Size:        {} bytes",
                        pvd.root.header.data_len.get()
                    );
                }
            }
            VolumeDescriptor::BootRecord(boot) => {
                println!();
                println!("Boot Record (El-Torito):");
                println!(
                    "  System ID:        {}",
                    text(&boot.boot_system_identifier).trim_end_matches(['\0', ' '])
                );
                println!("  Catalog Sector:   {}", boot.catalog_ptr.get());
            }
            VolumeDescriptor::Supplementary(svd) if svd.is_enhanced() => {
                println!();
                println!("Enhanced Volume Descriptor (ISO 9660:1999):");
                println!(
                    "  Volume ID:        {}",
                    text(svd.volume_identifier.trimmed())
                );
            }
            VolumeDescriptor::Supplementary(svd) => {
                println!();
                match JolietLevel::from_escape_sequences(&svd.escape_sequences) {
                    Some(level) => {
                        println!("Joliet Extension ({level:?}):");
                        println!(
                            "  Volume ID:        {}",
                            ucs2(svd.volume_identifier.as_bytes())
                        );
                    }
                    None => println!("Supplementary Volume Descriptor"),
                }
            }
            VolumeDescriptor::Terminator(_) => {}
            _ => {
                if args.verbose {
                    println!();
                    println!("Unknown Volume Descriptor");
                }
            }
        }
    }

    let mut catalog_buf = [0u8; 32 * 1024];
    let catalog = iso.boot_catalog(&mut catalog_buf)?;
    if let Some(catalog) = &catalog {
        println!();
        println!("Boot Catalog:");
        for entry in catalog.entries() {
            let emulation = entry
                .emulation()
                .map_or_else(|| "unknown".to_string(), |e| format!("{e:?}"));
            println!(
                "  {:?} {emulation}: block {}, {} sectors{}",
                entry.platform(),
                entry.load_block(),
                entry.sector_count(),
                if entry.is_bootable() {
                    ""
                } else {
                    " (not bootable)"
                }
            );
        }
    }

    let namespaces = iso.namespaces();
    let yes_no = |present: bool| if present { "Yes" } else { "No" };
    println!();
    println!("Features:");
    println!("  El-Torito Boot:   {}", yes_no(catalog.is_some()));
    println!(
        "  Joliet:           {}",
        yes_no(namespaces.contains(Namespace::Joliet))
    );
    println!(
        "  Rock Ridge:       {}",
        yes_no(namespaces.contains(Namespace::RockRidge))
    );
    if args.verbose {
        println!(
            "  Enhanced Tree:    {}",
            yes_no(namespaces.contains(Namespace::Enhanced))
        );
    }

    Ok(())
}

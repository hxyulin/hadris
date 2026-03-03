use std::fs::File;
use std::io::BufReader;

use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::IsoImage;
use hadris_iso::types::Endian;
use hadris_iso::volume::VolumeDescriptor;

use crate::args::InfoArgs;

use super::Result;

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
    let has_rockridge = iso.supports_rrip();

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
                println!(
                    "  Volume Size:      {} sectors ({} bytes)",
                    pvd.volume_space_size.read(),
                    pvd.volume_space_size.read() as u64 * 2048
                );
                println!(
                    "  Block Size:       {} bytes",
                    pvd.logical_block_size.read()
                );
                println!("  Path Table Size:  {} bytes", pvd.path_table_size.read());
                if args.verbose {
                    println!(
                        "  Root Extent:      sector {}",
                        pvd.dir_record.header.extent.read()
                    );
                    println!(
                        "  Root Size:        {} bytes",
                        pvd.dir_record.header.data_len.read()
                    );
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
    println!(
        "  El-Torito Boot:   {}",
        if has_boot { "Yes" } else { "No" }
    );
    println!(
        "  Joliet:           {}",
        if has_joliet { "Yes" } else { "No" }
    );
    println!(
        "  Rock Ridge:       {}",
        if has_rockridge { "Yes" } else { "No" }
    );

    Ok(())
}

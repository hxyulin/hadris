use std::fs::OpenOptions;

use hadris_iso::{joliet::JolietLevel, read::IsoImage, types::Endian, volume::VolumeDescriptor};
use tracing::Level;

use crate::args::ReadArgs;

pub fn read(args: ReadArgs) {
    let ReadArgs {
        input,
        verbose,
        display_info,
        extract,
        list,
    } = args;
    let subscriber = tracing_subscriber::fmt::SubscriberBuilder::default()
        .with_max_level(if verbose { Level::TRACE } else { Level::INFO })
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to set subscriber");

    let mut file = OpenOptions::new().read(true).open(input).unwrap();
    let sector_size = 2048;
    let iso = IsoImage::open(&mut file).unwrap();

    if display_info {
        println!("Volume Descriptors: ");
        for (idx, vd) in iso.read_volume_descriptors().enumerate() {
            let vd = vd.unwrap();
            print!("{idx}. ");
            match vd {
                VolumeDescriptor::Primary(pvd) => {
                    println!("PVD \"{}\"", pvd.volume_identifier);
                    let root_extent = pvd.dir_record.header.extent.read() as usize;
                    println!("    Root: {}", root_extent);
                }
                VolumeDescriptor::BootRecord(boot) => {
                    println!(
                        "BOOT \"{}\"",
                        core::str::from_utf8(&boot.boot_system_identifier)
                            .unwrap_or("<UNIDENTIFIED>")
                    );
                    let catalog_ptr = boot.catalog_ptr.get() as usize;
                    println!(
                        "    Catalog Ptr: {} (offset {:#x})",
                        catalog_ptr,
                        catalog_ptr * sector_size
                    );
                }
                VolumeDescriptor::Supplementary(svd) => match svd.file_structure_version {
                    1 => {
                        let mut ty = "<UNKNOWN>";
                        for jl in JolietLevel::all() {
                            if svd.escape_sequences == jl.escape_sequence() {
                                ty = "JOLIET";
                                break;
                            }
                        }

                        println!("SVD \"{}\" - {}", svd.volume_identifier, ty);
                    }
                    2 => {
                        println!("EVD \"{}\"", svd.volume_identifier);
                    }
                    _ => println!("UNKNOWN"),
                },
                _ => println!("UNKNOWN"),
            }
        }
    }

    let root = iso.root_dir();
    if let Some(path) = list {
        
    }
}

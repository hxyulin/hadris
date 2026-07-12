use std::fs::File;
use std::io::BufReader;

use hadris_iso::directory::FileFlags;
use hadris_iso::read::IsoImage;

use crate::args::LsArgs;

use super::{Result, navigate_to_path};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    let target = navigate_to_path(&iso, &args.path)?;
    let dir = iso.open_dir(target);

    for entry in dir.entries() {
        let entry = entry?;
        let name_bytes = entry.name();
        let name = String::from_utf8_lossy(name_bytes);

        // Handle special entries
        let display_name = match name_bytes {
            [0x00] => {
                if !args.all {
                    continue;
                }
                ".".to_string()
            }
            [0x01] => {
                if !args.all {
                    continue;
                }
                "..".to_string()
            }
            _ => {
                // Strip version number (;1) if present
                let name_str = name.to_string();
                if let Some(pos) = name_str.rfind(';') {
                    name_str[..pos].to_string()
                } else {
                    name_str
                }
            }
        };

        let flags = FileFlags::from_bits_truncate(entry.header().flags);

        if args.long {
            let type_char = if flags.contains(FileFlags::DIRECTORY) {
                'd'
            } else {
                '-'
            };
            let size = entry.header().data_len.read();
            let extent = entry.header().extent.read();

            println!(
                "{}  {:>10}  {:>8}  {}",
                type_char, size, extent, display_name
            );
        } else if flags.contains(FileFlags::DIRECTORY) {
            println!("{}/", display_name);
        } else {
            println!("{}", display_name);
        }
    }

    Ok(())
}

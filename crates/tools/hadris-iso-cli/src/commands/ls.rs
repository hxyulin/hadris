use std::fs::File;
use std::io::BufReader;

use hadris_iso::directory::FileFlags;
use hadris_iso::read::IsoImage;

use super::super::args::LsArgs;

use super::{Result, display_name, navigate_to_path};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;
    let entry_type = iso.root_dir().entry_type();

    let target = navigate_to_path(&iso, &args.path)?;
    let dir = iso.open_dir(target);

    for entry in dir.entries() {
        let entry = entry?;
        // Handle special entries
        let display_name = match entry.name() {
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
            _ => display_name(&entry, entry_type),
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

            println!("{type_char}  {size:>10}  {extent:>8}  {display_name}");
        } else if flags.contains(FileFlags::DIRECTORY) {
            println!("{display_name}/");
        } else {
            println!("{display_name}");
        }
    }

    Ok(())
}

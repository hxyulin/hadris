use std::fs::File;
use std::io::{self, BufReader, Write};

use hadris_iso::read::IsoImage;

use crate::args::CatArgs;

use super::{Result, clean_name, navigate_to_path};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let reader = BufReader::new(file);
    let iso = IsoImage::open(reader)?;

    // Split path into parent directory and filename
    let path = args.path.trim_start_matches('/');
    let (parent_path, filename) = if let Some(pos) = path.rfind('/') {
        (&path[..pos], &path[pos + 1..])
    } else {
        ("", path)
    };

    let parent_ref = navigate_to_path(&iso, parent_path)?;
    let dir = iso.open_dir(parent_ref);

    let file_entry = dir
        .entries()
        .filter_map(|e| e.ok())
        .find(|e| {
            if e.is_special() || e.is_directory() {
                return false;
            }
            let name = clean_name(e.name());
            name.eq_ignore_ascii_case(filename)
        })
        .ok_or_else(|| -> Box<dyn std::error::Error> {
            format!("File not found: {}", args.path).into()
        })?;

    let extent = file_entry.header().extent.read() as u64;
    let size = file_entry.header().data_len.read() as usize;

    if size > 0 {
        let mut buffer = vec![0u8; size];
        iso.read_bytes_at(extent * 2048, &mut buffer)?;
        io::stdout().write_all(&buffer)?;
    }

    Ok(())
}

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::CpioReader;

use super::{format_filetype, format_mode};

pub fn list(archive: PathBuf, long: bool) -> Result<()> {
    let file = File::open(&archive)
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    let mut reader = CpioReader::new(BufReader::new(file));

    while let Some(entry) = reader.next_entry_alloc().context("Failed to read entry")? {
        let name = entry.name_str().unwrap_or("<invalid utf-8>");
        let header = entry.header();

        if long {
            let ft_char = format_filetype(entry.file_type());
            let mode_str = format_mode(header.mode);
            let mtime = header.mtime;
            println!(
                "{}{} {:>5} {:>5} {:>8} {} {}",
                ft_char, mode_str, header.uid, header.gid, header.filesize, mtime, name,
            );
        } else {
            println!("{}", name);
        }

        reader
            .skip_entry_data_owned(&entry)
            .context("Failed to skip entry data")?;
    }

    Ok(())
}

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{format_filetype, format_mode, open_reader};

pub fn list(archive: PathBuf, long: bool) -> Result<()> {
    let mut reader = open_reader(&archive)?;

    while let Some(entry) = reader.next_entry().context("Failed to read entry")? {
        let name = entry.path_str().unwrap_or("<invalid utf-8>");
        if long {
            println!(
                "{}{} {:>5} {:>5} {:>8} {} {}",
                format_filetype(entry.file_type()),
                format_mode(entry.mode()),
                entry.uid(),
                entry.gid(),
                entry.len(),
                entry.mtime(),
                name,
            );
        } else {
            println!("{name}");
        }
    }

    Ok(())
}

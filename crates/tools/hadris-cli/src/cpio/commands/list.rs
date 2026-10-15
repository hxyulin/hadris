use std::path::PathBuf;

use anyhow::Result;

use super::{for_each_entry, format_mode, open_reader};
use crate::common::type_char;

pub fn list(archive: PathBuf, long: bool) -> Result<()> {
    let mut reader = open_reader(&archive)?;
    let mut current = 1;

    for_each_entry(&mut reader, |segment, entry| {
        if segment != current {
            current = segment;
            println!();
            println!("segment {segment}:");
        }
        let name = entry.path_str().unwrap_or("<invalid utf-8>");
        if long {
            println!(
                "{}{} {:>5} {:>5} {:>8} {} {}",
                type_char(entry.file_type()),
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
        Ok(())
    })?;

    Ok(())
}

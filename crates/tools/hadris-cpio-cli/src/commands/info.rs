use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::Format;

use super::{format_filetype, format_mode, format_size, open_reader};

pub fn info(archive: PathBuf) -> Result<()> {
    let mut reader = open_reader(&archive)?;

    let mut entry_count: u64 = 0;
    let mut total_data_size: u64 = 0;
    let mut format: Option<Format> = None;
    let mut details = Vec::new();

    while let Some(entry) = reader.next_entry().context("Failed to read entry")? {
        format.get_or_insert(entry.format());
        entry_count += 1;
        total_data_size += entry.len();
        details.push(format!(
            "  {}{} {}\n    ino={} nlink={} uid={} gid={} size={} mtime={}\n    dev={},{} rdev={},{} check={:#010x}",
            format_filetype(entry.file_type()),
            format_mode(entry.mode()),
            entry.name_str().unwrap_or("<invalid utf-8>"),
            entry.ino(),
            entry.nlink(),
            entry.uid(),
            entry.gid(),
            entry.len(),
            entry.mtime(),
            entry.dev().major(),
            entry.dev().minor(),
            entry.rdev().major(),
            entry.rdev().minor(),
            entry.check(),
        ));
    }

    let format_str = match format {
        Some(Format::Newc) => "newc (070701)",
        Some(Format::NewcCrc) => "newc+crc (070702)",
        Some(Format::Odc) => "odc (070707)",
        Some(Format::Binary) => "old binary",
        Some(_) => "unknown",
        None => "empty archive",
    };

    println!("CPIO Archive Information");
    println!("========================");
    println!("Format:       {format_str}");
    println!("Entries:      {entry_count}");
    println!(
        "Total data:   {} ({} bytes)",
        format_size(total_data_size),
        total_data_size
    );
    println!();

    if !details.is_empty() {
        println!("Entry Details");
        println!("-------------");
        for detail in &details {
            println!("{detail}");
        }
    }

    Ok(())
}

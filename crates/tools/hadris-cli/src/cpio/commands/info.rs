use std::path::PathBuf;

use anyhow::Result;
use hadris_cpio::Format;

use super::{for_each_entry, format_mode, open_reader};
use crate::common::{format_size, type_char};

fn format_name(format: Option<Format>) -> &'static str {
    match format {
        Some(Format::Newc) => "newc (070701)",
        Some(Format::Crc) => "newc+crc (070702)",
        Some(Format::Odc) => "odc (070707)",
        Some(Format::Binary) => "old binary",
        Some(_) => "unknown",
        None => "empty archive",
    }
}

pub fn info(archive: PathBuf) -> Result<()> {
    let mut reader = open_reader(&archive)?;

    let mut entry_count: u64 = 0;
    let mut total_data_size: u64 = 0;
    let mut segments: Vec<(Option<Format>, u64)> = Vec::new();
    let mut details = Vec::new();

    let count = for_each_entry(&mut reader, |segment, entry| {
        if segments.len() < segment {
            segments.resize(segment, (None, 0));
            if segment > 1 {
                details.push(format!("  segment {segment}:"));
            }
        }
        let (format, entries) = &mut segments[segment - 1];
        format.get_or_insert(entry.format());
        *entries += 1;
        entry_count += 1;
        total_data_size += entry.len();
        details.push(format!(
            "  {}{} {}\n    ino={} nlink={} uid={} gid={} size={} mtime={}\n    dev={},{} rdev={},{} check={:#010x}",
            type_char(entry.file_type()),
            format_mode(entry.mode()),
            entry.path_str().unwrap_or("<invalid utf-8>"),
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
        Ok(())
    })?;
    segments.resize(count.max(1), (None, 0));

    let formats: Vec<Format> = segments.iter().filter_map(|(format, _)| *format).collect();
    let format_str = if formats.windows(2).all(|pair| pair[0] == pair[1]) {
        format_name(formats.first().copied())
    } else {
        "mixed"
    };

    println!("CPIO Archive Information");
    println!("========================");
    println!("Format:       {format_str}");
    println!("Segments:     {}", segments.len());
    if segments.len() > 1 {
        for (index, (format, entries)) in segments.iter().enumerate() {
            println!(
                "  {}: {}, {entries} entries",
                index + 1,
                format_name(*format)
            );
        }
    }
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

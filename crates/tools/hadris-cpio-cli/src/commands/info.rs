use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::{CpioMagic, CpioReader};

use super::{format_filetype, format_mode, format_size};

pub fn info(archive: PathBuf) -> Result<()> {
    let file = File::open(&archive)
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    let mut reader = CpioReader::new(BufReader::new(file));

    let mut entry_count: u64 = 0;
    let mut total_data_size: u64 = 0;
    let mut format: Option<CpioMagic> = None;

    struct EntryInfo {
        name: String,
        ft_char: char,
        mode_str: String,
        ino: u32,
        uid: u32,
        gid: u32,
        nlink: u32,
        mtime: u32,
        filesize: u32,
        devmajor: u32,
        devminor: u32,
        rdevmajor: u32,
        rdevminor: u32,
        check: u32,
    }

    let mut entries = Vec::new();

    while let Some(entry) = reader.next_entry_alloc().context("Failed to read entry")? {
        let name = entry.name_str().unwrap_or("<invalid utf-8>").to_string();
        let header = entry.header().clone();

        if format.is_none() {
            format = Some(entry.magic());
        }

        entry_count += 1;
        total_data_size += header.filesize as u64;

        entries.push(EntryInfo {
            name,
            ft_char: format_filetype(entry.file_type()),
            mode_str: format_mode(header.mode),
            ino: header.ino,
            uid: header.uid,
            gid: header.gid,
            nlink: header.nlink,
            mtime: header.mtime,
            filesize: header.filesize,
            devmajor: header.devmajor,
            devminor: header.devminor,
            rdevmajor: header.rdevmajor,
            rdevminor: header.rdevminor,
            check: header.check,
        });

        reader
            .skip_entry_data_owned(&entry)
            .context("Failed to skip entry data")?;
    }

    let format_str = match format {
        Some(CpioMagic::Newc) => "newc (070701)",
        Some(CpioMagic::NewcCrc) => "newc+crc (070702)",
        None => "empty archive",
    };

    println!("CPIO Archive Information");
    println!("========================");
    println!("Format:       {}", format_str);
    println!("Entries:      {}", entry_count);
    println!(
        "Total data:   {} ({} bytes)",
        format_size(total_data_size),
        total_data_size
    );
    println!();

    if !entries.is_empty() {
        println!("Entry Details");
        println!("-------------");
        for e in &entries {
            println!("  {}{} {}", e.ft_char, e.mode_str, e.name);
            println!(
                "    ino={} nlink={} uid={} gid={} size={} mtime={}",
                e.ino, e.nlink, e.uid, e.gid, e.filesize, e.mtime,
            );
            println!(
                "    dev={},{} rdev={},{} check={:#010x}",
                e.devmajor, e.devminor, e.rdevmajor, e.rdevminor, e.check,
            );
        }
    }

    Ok(())
}

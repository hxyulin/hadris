//! Print a canonical listing of a filesystem/partition image for differential
//! testing against reference tools.
//!
//! Usage: `fs_dump <fat|exfat|ntfs|iso|udf|cpio|part> <image-path>`
//!
//! One line per entry, sorted:
//!   `file <size> <fnv1a64-of-first-4KiB> <path>` for files
//!   `dir <path>` for directories
//!   `<index> <start_lba> <size_sectors>` per partition (part)
//!
//! On mount/parse failure (or panic) print nothing and exit 0 — differential
//! testing only compares images both sides can mount.

use std::io::Cursor;

const DEPTH_CAP: u32 = 64;
const ENTRY_BUDGET: u32 = 200_000;
const CONTENT_CAP: usize = 4096;

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn file_line(size: u64, content: &[u8], path: &str) -> String {
    format!("file {} {:016x} {}", size, fnv1a64(content), path)
}

// Read up to CONTENT_CAP bytes via a sync reader's inherent `read`.
macro_rules! read_head {
    ($reader:expr) => {{
        let mut buf = [0u8; CONTENT_CAP];
        let mut filled = 0usize;
        while filled < CONTENT_CAP {
            match $reader.read(&mut buf[filled..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => filled += n,
            }
        }
        buf[..filled].to_vec()
    }};
}

fn dump_fat(data: &[u8]) -> Vec<String> {
    use hadris_fat::{FatVolume, FatVolumeReadExt};

    let mut lines = Vec::new();
    let Ok(fs) = FatVolume::open(Cursor::new(data)) else {
        return lines;
    };
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(fs.root_dir(), String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        for item in dir.entries() {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let Ok(de) = item else { continue };
            let Some(fe) = de.as_entry() else { continue };
            let name = fe.name();
            if name == "." || name == ".." {
                continue;
            }
            let child_path = format!("{path}{name}");
            if fe.is_directory() {
                lines.push(format!("dir {child_path}"));
                if let Ok(child) = dir.open_entry(fe) {
                    stack.push((child, format!("{child_path}/"), depth + 1));
                }
            } else {
                let content = match fs.read_file(fe) {
                    Ok(mut reader) => read_head!(reader),
                    Err(_) => Vec::new(),
                };
                lines.push(file_line(fe.len(), &content, &child_path));
            }
        }
    }
    lines
}

fn dump_exfat(data: &[u8]) -> Vec<String> {
    use hadris_fat::exfat::{ExFatFileReader, ExFatVolume};
    use hadris_fat::io::Read;

    let mut lines = Vec::new();
    let Ok(fs) = ExFatVolume::open(Cursor::new(data)) else {
        return lines;
    };
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(fs.root_dir(), String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        for item in dir.entries() {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let Ok(entry) = item else { continue };
            let child_path = format!("{path}{}", entry.name);
            if entry.is_directory() {
                lines.push(format!("dir {child_path}"));
                if let Ok(child) = dir.open_dir(&entry.name) {
                    stack.push((child, format!("{child_path}/"), depth + 1));
                }
            } else {
                let content = match ExFatFileReader::new(&fs, &entry) {
                    Ok(mut reader) => read_head!(reader),
                    Err(_) => Vec::new(),
                };
                lines.push(file_line(entry.size(), &content, &child_path));
            }
        }
    }
    lines
}

fn dump_ntfs(data: &[u8]) -> Vec<String> {
    use hadris_ntfs::sync::{NtfsFs, NtfsFsReadExt};

    let mut lines = Vec::new();
    let Ok(fs) = NtfsFs::open(Cursor::new(data)) else {
        return lines;
    };
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(fs.root_dir(), String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        let Ok(entries) = dir.entries() else { continue };
        for entry in entries {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let child_path = format!("{path}{}", entry.name());
            if entry.is_directory() {
                lines.push(format!("dir {child_path}"));
                if let Ok(child) = dir.open_dir(entry.name()) {
                    stack.push((child, format!("{child_path}/"), depth + 1));
                }
            } else {
                let content = match fs.read_file(&entry) {
                    Ok(mut reader) => read_head!(reader),
                    Err(_) => Vec::new(),
                };
                lines.push(file_line(entry.size(), &content, &child_path));
            }
        }
    }
    lines
}

fn dump_iso(data: &[u8]) -> Vec<String> {
    use hadris_iso::read::IsoImage;

    let mut lines = Vec::new();
    let Ok(image) = IsoImage::open(Cursor::new(data)) else {
        return lines;
    };
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(image.root_dir().dir_ref(), String::from("/"), 0u32)];
    while let Some((dref, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        let dir = image.open_dir(dref);
        for item in dir.entries() {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let Ok(entry) = item else { continue };
            if entry.is_special() {
                continue;
            }
            let child_path = format!("{path}{}", entry.display_name());
            if entry.is_directory() {
                lines.push(format!("dir {child_path}"));
                if let Ok(child) = entry.as_dir_ref(&image) {
                    stack.push((child, format!("{child_path}/"), depth + 1));
                }
            } else {
                let content = match image.read_file(&entry) {
                    Ok(bytes) => bytes,
                    Err(_) => Vec::new(),
                };
                let head = &content[..content.len().min(CONTENT_CAP)];
                lines.push(file_line(entry.total_size(), head, &child_path));
            }
        }
    }
    lines
}

fn dump_udf(data: &[u8]) -> Vec<String> {
    use hadris_udf::UdfVolume;

    let mut lines = Vec::new();
    let Ok(fs) = UdfVolume::open(Cursor::new(data)) else {
        return lines;
    };
    let Ok(root) = fs.root_dir() else {
        return lines;
    };
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(root, String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        for entry in dir.entries() {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            if entry.is_parent() || entry.name().is_empty() {
                continue;
            }
            let child_path = format!("{path}{}", entry.name());
            if entry.is_dir() {
                lines.push(format!("dir {child_path}"));
                if let Ok(child) = fs.read_directory(&entry.icb) {
                    stack.push((child, format!("{child_path}/"), depth + 1));
                }
            } else {
                let content = match fs.read_file(entry) {
                    Ok(bytes) => bytes,
                    Err(_) => Vec::new(),
                };
                let head = &content[..content.len().min(CONTENT_CAP)];
                lines.push(file_line(entry.size, head, &child_path));
            }
        }
    }
    lines
}

fn dump_cpio(data: &[u8]) -> Vec<String> {
    use hadris_cpio::mode::FileType;
    use hadris_cpio::sync::CpioArchiveReader;

    let mut lines = Vec::new();
    let mut budget = ENTRY_BUDGET;
    let mut reader = CpioArchiveReader::new(Cursor::new(data));
    while let Ok(Some(entry)) = reader.next_entry_alloc() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let name = String::from_utf8_lossy(entry.name()).into_owned();
        let content = reader.read_entry_data_alloc(&entry).unwrap_or_default();
        match entry.file_type() {
            FileType::Directory => lines.push(format!("dir {name}")),
            _ => {
                let head = &content[..content.len().min(CONTENT_CAP)];
                lines.push(file_line(u64::from(entry.file_size()), head, &name));
            }
        }
    }
    lines
}

fn dump_part(data: &[u8]) -> Vec<String> {
    use hadris_part::{PartitionTable, PartitionTableReadExt};

    let mut lines = Vec::new();
    let mut cursor = Cursor::new(data);
    let Ok(table) = PartitionTable::read_from(&mut cursor, 512) else {
        return lines;
    };
    for partition in table.partitions() {
        lines.push(format!(
            "{} {} {}",
            partition.index, partition.start_lba, partition.size_sectors
        ));
    }
    lines
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: fs_dump <fat|exfat|ntfs|iso|udf|cpio|part> <image-path>");
        std::process::exit(2);
    }
    let format = args[1].clone();
    if !matches!(
        format.as_str(),
        "fat" | "exfat" | "ntfs" | "iso" | "udf" | "cpio" | "part"
    ) {
        eprintln!("unknown format: {format}");
        std::process::exit(2);
    }
    let Ok(data) = std::fs::read(&args[2]) else {
        std::process::exit(0);
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        match format.as_str() {
            "fat" => dump_fat(&data),
            "exfat" => dump_exfat(&data),
            "ntfs" => dump_ntfs(&data),
            "iso" => dump_iso(&data),
            "udf" => dump_udf(&data),
            "cpio" => dump_cpio(&data),
            _ => dump_part(&data),
        }
    }));
    let Ok(mut lines) = result else {
        eprintln!("fs_dump: panicked on input");
        std::process::exit(0);
    };
    lines.sort();
    lines.dedup();
    for line in lines {
        println!("{line}");
    }
}

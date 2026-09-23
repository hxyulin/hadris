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

use hadris_io::Cursor;

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

/// Lists any `hadris-fs` driver through the node API.
fn dump_driver<D: hadris_fs::sync::FsDriver>(fs: &mut D) -> Vec<String> {
    use hadris_fs::{DirCursor, FileType, NameBuf};

    let mut lines = Vec::new();
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(fs.root(), String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        if depth > DEPTH_CAP {
            continue;
        }
        let mut cursor = DirCursor::start();
        let mut name = NameBuf::new();
        while let Ok(Some(entry)) = fs.read_dir_entry(dir, &mut cursor, &mut name) {
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let Some(text) = name.as_name().and_then(|n| n.to_str().ok()) else {
                continue;
            };
            let child_path = format!("{path}{text}");
            match entry.file_type() {
                FileType::Dir => {
                    lines.push(format!("dir {child_path}"));
                    stack.push((entry.node(), format!("{child_path}/"), depth + 1));
                }
                FileType::File => {
                    let size = fs.node_metadata(entry.node()).map_or(0, |meta| meta.len());
                    let mut buf = [0u8; CONTENT_CAP];
                    let mut filled = 0usize;
                    while filled < CONTENT_CAP {
                        match fs.read_at(entry.node(), filled as u64, &mut buf[filled..]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => filled += n,
                        }
                    }
                    lines.push(file_line(size, &buf[..filled], &child_path));
                }
                _ => {}
            }
        }
    }
    lines
}

fn dump_fat(data: &[u8]) -> Vec<String> {
    use hadris_fat::sync::FatFs;
    use hadris_fat::MountOptions;
    use hadris_fs::HeapTable;
    use hadris_storage::{BlockSize, MemDevice};

    let dev = MemDevice::new(data, BlockSize::new(512).unwrap());
    let options = MountOptions::new()
        .with_read_only()
        .with_table(HeapTable::new());
    match FatFs::open_with(dev, options) {
        Ok(mut fs) => dump_driver(&mut fs),
        Err(_) => Vec::new(),
    }
}

fn dump_exfat(data: &[u8]) -> Vec<String> {
    use hadris_fat::exfat::{ExFatFileReader, ExFatVolume};
    use hadris_io::legacy::sync::Read;

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
                let content = image.read_file(&entry).unwrap_or_default();
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
                let content = fs.read_file(entry).unwrap_or_default();
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
    use hadris_storage::{BlockSize, MemDevice};

    let mut lines = Vec::new();
    let mut dev = MemDevice::new(data, BlockSize::new(512).unwrap());
    let Ok(disk) = hadris_part::sync::read(&mut dev) else {
        return lines;
    };
    for partition in disk.partitions() {
        lines.push(format!(
            "{} {} {}",
            partition.index(),
            partition.start(),
            partition.len()
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

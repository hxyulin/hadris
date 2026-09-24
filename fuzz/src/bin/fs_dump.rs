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

use hadris_fs::sync::FileSystem;
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

/// Lists any filesystem through the shared `FileSystem` node API, so this
/// one walk serves every format below.
fn dump<F: FileSystem>(fs: &mut F) -> Vec<String> {
    use hadris_fs::{DirCursor, FileType, OpenMode};

    let mut lines = Vec::new();
    let mut budget = ENTRY_BUDGET;
    let mut stack = vec![(fs.root(), String::from("/"), 0u32)];
    while let Some((dir, path, depth)) = stack.pop() {
        let mut cursor = DirCursor::START;
        while depth <= DEPTH_CAP {
            let Ok(Some(entry)) = fs.readdir(dir, cursor) else {
                break;
            };
            cursor = entry.next_cursor();
            if budget == 0 {
                return lines;
            }
            budget -= 1;
            let Ok(text) = entry.name().to_str() else {
                continue;
            };
            let child_path = format!("{path}{text}");
            let Ok(child) = fs.lookup(dir, entry.name()) else {
                continue;
            };
            match entry.file_type() {
                FileType::Dir => {
                    lines.push(format!("dir {child_path}"));
                    stack.push((child, format!("{child_path}/"), depth + 1));
                    continue;
                }
                FileType::File => {
                    let size = entry.metadata().len();
                    let mut buf = [0u8; CONTENT_CAP];
                    let mut filled = 0usize;
                    if fs.open(child, OpenMode::Read).is_ok() {
                        while filled < CONTENT_CAP {
                            match fs.read(child, filled as u64, &mut buf[filled..]) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => filled += n,
                            }
                        }
                        let _ = fs.close(child);
                    }
                    lines.push(file_line(size, &buf[..filled], &child_path));
                }
                _ => {}
            }
            fs.forget(child, 1);
        }
        fs.forget(dir, 1);
    }
    lines
}

/// Lists a mounted filesystem with [`dump`].
fn dump_driver<F: FileSystem>(mut fs: F) -> Vec<String> {
    dump(&mut fs)
}

fn dump_fat(data: &[u8]) -> Vec<String> {
    use hadris_fat::sync::FatFs;
    use hadris_fs::MountOptions;
    use hadris_storage::{BlockSize, MemDevice};

    let dev = MemDevice::new(data, BlockSize::new(512).unwrap());
    let options = MountOptions::new().read_only();
    match FatFs::mount(dev, options) {
        Ok(fs) => dump_driver(fs),
        Err(_) => Vec::new(),
    }
}

fn dump_exfat(data: &[u8]) -> Vec<String> {
    use hadris_fat::exfat::sync::ExFatFs;
    use hadris_fs::MountOptions;
    use hadris_storage::{BlockSize, MemDevice};

    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let dev = MemDevice::new(bytes, BlockSize::new(512).unwrap());
    let options = MountOptions::new().read_only();
    match ExFatFs::mount(dev, options) {
        Ok(fs) => dump_driver(fs),
        Err(_) => Vec::new(),
    }
}

fn dump_ntfs(data: &[u8]) -> Vec<String> {
    use hadris_ntfs::sync::NtfsFs;
    use hadris_storage::{BlockSize, MemDevice};

    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    match NtfsFs::open(MemDevice::new(bytes, BlockSize::new(512).unwrap())) {
        Ok(fs) => dump_driver(fs),
        Err(_) => Vec::new(),
    }
}

fn dump_iso(data: &[u8]) -> Vec<String> {
    use hadris_iso::sync::IsoImage;
    use hadris_iso::Namespace;
    use hadris_storage::{BlockSize, MemDevice};

    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let dev = MemDevice::new(bytes, BlockSize::new(512).unwrap());
    let Ok(image) = IsoImage::open(dev) else {
        return Vec::new();
    };
    match image.into_view(Namespace::Preferred) {
        Ok(view) => dump_driver(view),
        Err(_) => Vec::new(),
    }
}

fn dump_udf(data: &[u8]) -> Vec<String> {
    use hadris_storage::{BlockSize, MemDevice};
    use hadris_udf::sync::UdfFs;

    let mut bytes = data.to_vec();
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    match UdfFs::open(MemDevice::new(bytes, BlockSize::new(512).unwrap())) {
        Ok(fs) => dump_driver(fs),
        Err(_) => Vec::new(),
    }
}

fn dump_cpio(data: &[u8]) -> Vec<String> {
    use hadris_cpio::sync::CpioReader;
    use hadris_fs::FileType;
    use hadris_io::sync::Read;

    let mut lines = Vec::new();
    let mut budget = ENTRY_BUDGET;
    let mut reader = CpioReader::new(Cursor::new(data));
    while let Ok(Some(mut entry)) = reader.next_entry() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let name = String::from_utf8_lossy(entry.name()).into_owned();
        match entry.file_type() {
            FileType::Dir => lines.push(format!("dir {name}")),
            _ => {
                let mut head = vec![0u8; CONTENT_CAP];
                let mut filled = 0;
                while filled < head.len() {
                    match entry.read(&mut head[filled..]) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => filled += read,
                    }
                }
                lines.push(file_line(entry.len(), &head[..filled], &name));
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

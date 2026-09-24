mod cat;
mod create;
mod extract;
mod info;
mod list;

pub use cat::cat;
pub use create::create;
pub use extract::extract;
pub use info::info;
pub use list::list;

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use hadris_cpio::sync::CpioReader;
use hadris_fs::FileType;
use hadris_io::StdIo;

/// The archive at `path`, or standard input for `-`.
pub type Input = StdIo<BufReader<Box<dyn Read>>>;

fn open_reader(path: &Path) -> Result<CpioReader<Input>> {
    let input: Box<dyn Read> = if path.as_os_str() == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(
            File::open(path)
                .with_context(|| format!("Failed to open archive: {}", path.display()))?,
        )
    };
    Ok(CpioReader::new(StdIo::new(BufReader::new(input))))
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_mode(mode: u32) -> String {
    let perms = mode & 0o7777;
    let mut s = String::with_capacity(9);
    for &(bit, ch) in &[
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        s.push(if perms & bit != 0 { ch } else { '-' });
    }
    s
}

fn format_filetype(ft: FileType) -> char {
    match ft {
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        FileType::File => '-',
        FileType::CharDevice => 'c',
        FileType::BlockDevice => 'b',
        FileType::Fifo => 'p',
        FileType::Socket => 's',
        _ => '?',
    }
}

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
use hadris_cpio::Detail;
use hadris_cpio::sync::{CpioReader, Entry};
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
    Ok(reader(input))
}

fn reader(input: Box<dyn Read>) -> CpioReader<Input> {
    CpioReader::new(StdIo::new(BufReader::new(input)))
}

/// Calls `each` with every entry of every concatenated segment, as the
/// kernel reads an initramfs, and the segment's number, counting from 1.
/// Data after a trailer that does not start with a cpio header, such as a
/// compressed initramfs after an uncompressed microcode archive, ends the
/// archive with a warning. Returns the number of segments read.
fn for_each_entry(
    reader: &mut CpioReader<Input>,
    mut each: impl FnMut(usize, &mut Entry<'_, Input>) -> Result<()>,
) -> Result<usize> {
    let mut segment = 1;
    loop {
        let start = reader.offset().saturating_sub(1);
        let mut first = true;
        loop {
            let mut entry = match reader.next_entry() {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(err) if first && segment > 1 && Detail::of(&err) == Some(Detail::Magic) => {
                    eprintln!(
                        "warning: ignoring data at byte {start} after segment {}: not a cpio archive",
                        segment - 1
                    );
                    return Ok(segment - 1);
                }
                Err(err) => return Err(anyhow::Error::new(err).context("Failed to read entry")),
            };
            first = false;
            each(segment, &mut entry)?;
        }
        if !reader.next_segment().context("Failed to read archive")? {
            return Ok(segment);
        }
        segment += 1;
    }
}

/// The key of a hard link group: device major and minor, and inode.
type GroupKey = (u32, u32, u64);

fn link_key(entry: &Entry<'_, Input>) -> GroupKey {
    let dev = entry.dev();
    (dev.major(), dev.minor(), entry.ino())
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

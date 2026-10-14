use std::fs::File;
use std::io::{self, Read as _, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_fs::FileType;
use hadris_io::sync::Read;

use super::{GroupKey, for_each_entry, link_key, open_reader};

/// The data of the last entry of the requested name: its place in the
/// archive file, or a copy when the archive is standard input.
enum Found {
    At { offset: u64, len: u64 },
    Copy(File),
}

pub fn cat(archive: PathBuf, path: &str) -> Result<()> {
    let mut reader = open_reader(&archive)?;
    let seekable =
        archive.as_os_str() != "-" && std::fs::metadata(&archive).is_ok_and(|meta| meta.is_file());
    let want = normalize(path.as_bytes());
    let mut found = None;
    let mut link: Option<(usize, GroupKey)> = None;
    let mut buf = vec![0u8; 64 * 1024];

    for_each_entry(&mut reader, |segment, entry| {
        let is_file = entry.file_type() == FileType::File;
        if normalize(entry.path()) == want {
            link = None;
            if is_file && entry.len() == 0 && entry.nlink() > 1 {
                link = Some((segment, link_key(entry)));
            }
        } else if !is_file || entry.len() == 0 || link != Some((segment, link_key(entry))) {
            return Ok(());
        }
        if seekable {
            found = Some(Found::At {
                offset: entry.data_offset(),
                len: entry.len(),
            });
            return Ok(());
        }
        let mut copy = tempfile::tempfile().context("Failed to create a temporary file")?;
        loop {
            let read = entry.read(&mut buf).context("Failed to read entry data")?;
            if read == 0 {
                break;
            }
            copy.write_all(&buf[..read])
                .context("Failed to write a temporary file")?;
        }
        found = Some(Found::Copy(copy));
        Ok(())
    })?;

    let mut stdout = io::stdout().lock();
    let copied = match found {
        None => bail!("File not found in archive: {path}"),
        Some(Found::At { offset, len }) => {
            let mut file = File::open(&archive)
                .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
            file.seek(SeekFrom::Start(offset))
                .context("Failed to read entry data")?;
            io::copy(&mut file.take(len), &mut stdout)
        }
        Some(Found::Copy(mut copy)) => {
            copy.seek(SeekFrom::Start(0))
                .context("Failed to read entry data")?;
            io::copy(&mut copy, &mut stdout)
        }
    };
    copied.context("Failed to write to stdout")?;
    Ok(())
}

/// `path` without empty and `.` components, so `/a/b`, `./a/b` and `a/b`
/// name the same entry, as in the other formats' commands.
fn normalize(path: &[u8]) -> Vec<&[u8]> {
    path.split(|&b| b == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .collect()
}

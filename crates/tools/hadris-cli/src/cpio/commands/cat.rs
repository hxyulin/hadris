use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_cpio::sync::CpioReader;
use hadris_fs::FileType;
use hadris_io::sync::Read;

use super::{GroupKey, Input, for_each_entry, link_key, reader};

/// Where entry data is: offset and length in the archive file, or in the
/// spool when the archive is standard input.
type Place = (u64, u64);

pub fn cat(archive: PathBuf, path: &str) -> Result<()> {
    let (mut reader, file) = open(&archive)?;
    let mut spool = match file {
        Some(_) => None,
        None => Some(tempfile::tempfile().context("Failed to create a temporary file")?),
    };
    let want = normalize(path.as_bytes());
    let mut found = None;
    let mut link: Option<(usize, GroupKey)> = None;
    let mut with_data: HashMap<(usize, GroupKey), Place> = HashMap::new();
    let mut buf = vec![0u8; 64 * 1024];

    for_each_entry(&mut reader, |segment, entry| {
        let is_file = entry.file_type() == FileType::File;
        let group = (segment, link_key(entry));
        let named = normalize(entry.path()) == want;
        let data_link = is_file && !entry.is_empty() && entry.nlink() > 1;
        let follows = !named && is_file && !entry.is_empty() && link == Some(group);
        if !named && !follows && !data_link {
            return Ok(());
        }
        let place = match &mut spool {
            None => (entry.data_offset(), entry.len()),
            Some(spool) => append(spool, entry, &mut buf)?,
        };
        if data_link {
            with_data.insert(group, place);
        }
        if named {
            link = None;
            found = Some(place);
            if is_file && entry.is_empty() && entry.nlink() > 1 {
                link = Some(group);
                if let Some(&data) = with_data.get(&group) {
                    found = Some(data);
                }
            }
        } else if follows {
            found = Some(place);
        }
        Ok(())
    })?;

    let Some((offset, len)) = found else {
        bail!("File not found in archive: {path}");
    };
    let source = spool.or(file).context("Failed to read entry data")?;
    copy_data(source, offset, len, &mut io::stdout().lock())
}

/// A reader of the archive at `archive`, or of standard input for `-`,
/// and, when it is a regular file, another handle to the file it reads.
fn open(archive: &Path) -> Result<(CpioReader<Input>, Option<File>)> {
    if archive.as_os_str() == "-" {
        return Ok((reader(Box::new(io::stdin())), None));
    }
    let file = File::open(archive)
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    let is_file = file
        .metadata()
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?
        .is_file();
    if !is_file {
        return Ok((reader(Box::new(file)), None));
    }
    let scan = file
        .try_clone()
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    Ok((reader(Box::new(scan)), Some(file)))
}

/// Writes the `len` bytes at `offset` of `source` to `out`, failing when
/// `source` ends first.
fn copy_data(mut source: File, offset: u64, len: u64, out: &mut impl Write) -> Result<()> {
    source
        .seek(SeekFrom::Start(offset))
        .context("Failed to read entry data")?;
    let copied = io::copy(&mut source.take(len), out).context("Failed to write to stdout")?;
    if copied != len {
        bail!("Archive ends {copied} bytes into entry data of {len} bytes");
    }
    Ok(())
}

/// Copies the data of `entry` to the end of `spool` and returns where.
fn append(
    spool: &mut File,
    entry: &mut hadris_cpio::sync::Entry<'_, Input>,
    buf: &mut [u8],
) -> Result<Place> {
    let start = spool
        .seek(SeekFrom::End(0))
        .context("Failed to write a temporary file")?;
    loop {
        let read = entry.read(buf).context("Failed to read entry data")?;
        if read == 0 {
            break;
        }
        spool
            .write_all(&buf[..read])
            .context("Failed to write a temporary file")?;
    }
    let end = spool
        .stream_position()
        .context("Failed to write a temporary file")?;
    Ok((start, end - start))
}

/// `path` without empty and `.` components, so `/a/b`, `./a/b` and `a/b`
/// name the same entry, as in the other formats' commands.
fn normalize(path: &[u8]) -> Vec<&[u8]> {
    path.split(|&b| b == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_entry_data_fails() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"0123456789").unwrap();
        let mut out = Vec::new();
        copy_data(file.try_clone().unwrap(), 4, 3, &mut out).unwrap();
        assert_eq!(out, b"456");
        out.clear();
        let err = copy_data(file, 4, 10, &mut out).unwrap_err();
        assert!(err.to_string().contains("6 bytes into"), "{err}");
    }
}

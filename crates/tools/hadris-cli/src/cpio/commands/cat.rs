use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read as _, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_fs::FileType;
use hadris_io::sync::Read;

use super::{GroupKey, Input, for_each_entry, link_key, open_reader};

/// Where entry data is: offset and length in the archive file, or in the
/// spool when the archive is standard input.
type Place = (u64, u64);

pub fn cat(archive: PathBuf, path: &str) -> Result<()> {
    let mut reader = open_reader(&archive)?;
    let seekable =
        archive.as_os_str() != "-" && std::fs::metadata(&archive).is_ok_and(|meta| meta.is_file());
    let mut spool = match seekable {
        true => None,
        false => Some(tempfile::tempfile().context("Failed to create a temporary file")?),
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
        let data_link = is_file && entry.len() > 0 && entry.nlink() > 1;
        let follows = !named && is_file && entry.len() > 0 && link == Some(group);
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
            if is_file && entry.len() == 0 && entry.nlink() > 1 {
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
    let mut source = match spool {
        Some(spool) => spool,
        None => File::open(&archive)
            .with_context(|| format!("Failed to open archive: {}", archive.display()))?,
    };
    source
        .seek(SeekFrom::Start(offset))
        .context("Failed to read entry data")?;
    io::copy(&mut source.take(len), &mut io::stdout().lock())
        .context("Failed to write to stdout")?;
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

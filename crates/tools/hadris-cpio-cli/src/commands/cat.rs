use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_io::sync::Read;

use super::open_reader;

pub fn cat(archive: PathBuf, path: &str) -> Result<()> {
    let mut reader = open_reader(&archive)?;
    let mut link = None;

    while let Some(mut entry) = reader.next_entry().context("Failed to read entry")? {
        let wanted = entry.name() == path.as_bytes() || link == Some(entry.ino());
        if !wanted {
            continue;
        }
        if entry.len() == 0 && entry.nlink() > 1 {
            link = Some(entry.ino());
            continue;
        }
        let mut stdout = io::stdout().lock();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buf).context("Failed to read entry data")?;
            if read == 0 {
                break;
            }
            stdout
                .write_all(&buf[..read])
                .context("Failed to write to stdout")?;
        }
        return Ok(());
    }

    if link.is_some() {
        return Ok(());
    }
    bail!("File not found in archive: {path}")
}

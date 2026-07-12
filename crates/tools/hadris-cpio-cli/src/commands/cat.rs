use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_cpio::CpioReader;

pub fn cat(archive: PathBuf, path: &str) -> Result<()> {
    let file = File::open(&archive)
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    let mut reader = CpioReader::new(BufReader::new(file));

    while let Some(entry) = reader.next_entry_alloc().context("Failed to read entry")? {
        let name = entry.name_str().unwrap_or("");

        if name == path {
            let data = reader
                .read_entry_data_alloc(&entry)
                .context("Failed to read entry data")?;
            io::stdout()
                .write_all(&data)
                .context("Failed to write to stdout")?;
            return Ok(());
        }

        reader.skip_entry_data_owned(&entry)?;
    }

    bail!("File not found in archive: {}", path)
}

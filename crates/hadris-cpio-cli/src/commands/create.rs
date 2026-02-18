use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::{CpioWriteOptions, CpioWriter, FileTree};

pub fn create(directory: PathBuf, output: PathBuf, crc: bool) -> Result<()> {
    let tree = FileTree::from_fs(&directory)
        .with_context(|| format!("Failed to scan directory: {}", directory.display()))?;

    let options = CpioWriteOptions { use_crc: crc };
    let writer = CpioWriter::new(options);

    let file = File::create(&output)
        .with_context(|| format!("Failed to create output file: {}", output.display()))?;
    let mut buf = BufWriter::new(file);

    writer
        .write(&mut buf, &tree)
        .context("Failed to write CPIO archive")?;

    let format_name = if crc { "newc+crc" } else { "newc" };
    println!(
        "Created {} archive: {}",
        format_name,
        output.display()
    );

    Ok(())
}

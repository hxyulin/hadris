use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_cpio::{CpioArchiveWriter, CpioWriteOptions, FileTree};

fn main() -> Result<()> {
    let (source_path, archive_path) = arguments()?;
    let tree = FileTree::from_fs(&source_path)
        .with_context(|| format!("failed to scan {}", source_path.display()))?;
    let output = File::create(&archive_path)
        .with_context(|| format!("failed to create {}", archive_path.display()))?;
    let output = BufWriter::new(output);

    CpioArchiveWriter::new(output, CpioWriteOptions::default())
        .finish(&tree)
        .with_context(|| format!("failed to write {}", archive_path.display()))?;

    println!("created {}", archive_path.display());
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf)> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(source) = args.next() else {
        bail!(
            "usage: {} <source-directory> <archive.cpio>",
            PathBuf::from(program).display()
        );
    };
    let Some(archive) = args.next() else {
        bail!("missing output archive path");
    };
    if args.next().is_some() {
        bail!("too many arguments");
    }
    Ok((source.into(), archive.into()))
}

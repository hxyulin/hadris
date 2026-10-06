use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_cpio::CpioOptions;
use hadris_fs::host::{self, TreeOptions};
use hadris_io::StdIo;

fn main() -> Result<()> {
    let (source_path, archive_path) = arguments()?;
    let (tree, _) = host::read_tree(&source_path, &TreeOptions::new())
        .with_context(|| format!("failed to scan {}", source_path.display()))?;
    let output = File::create(&archive_path)
        .with_context(|| format!("failed to create {}", archive_path.display()))?;
    let mut output = StdIo::new(BufWriter::new(output));

    hadris_cpio::sync::write(&mut output, &tree, &CpioOptions::default())
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

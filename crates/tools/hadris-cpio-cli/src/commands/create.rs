use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::tree::{FromFsOptions, OnError, Tree, WarningKind};
use hadris_io::StdIo;

use crate::app::ArchiveFormat;

pub fn create(
    directory: PathBuf,
    output: PathBuf,
    format: ArchiveFormat,
    verbose: bool,
) -> Result<()> {
    let tree = Tree::from_fs(
        &directory,
        FromFsOptions::new().with_on_error(OnError::Warn),
    )
    .with_context(|| format!("Failed to scan directory: {}", directory.display()))?;
    for warning in tree.warnings() {
        eprintln!("warning: skipped {warning}");
    }

    let (format, name) = match format {
        ArchiveFormat::Newc => (Format::Newc, "newc"),
        ArchiveFormat::Crc => (Format::NewcCrc, "newc+crc"),
        ArchiveFormat::Odc => (Format::Odc, "odc"),
    };
    let options = CpioOptions::default().with_format(format);
    let to_stdout = output.as_os_str() == "-";
    let sink: Box<dyn Write> = if to_stdout {
        Box::new(io::stdout().lock())
    } else {
        Box::new(
            File::create(&output)
                .with_context(|| format!("Failed to create output file: {}", output.display()))?,
        )
    };
    let mut out = StdIo::new(BufWriter::new(sink));
    let report = hadris_cpio::sync::write(&mut out, &tree, &options)
        .context("Failed to write CPIO archive")?;

    for warning in report.warnings() {
        if verbose || warning.kind() != WarningKind::IgnoredMetadata {
            eprintln!("warning: {warning}");
        }
    }
    if !to_stdout {
        println!("Created {name} archive: {}", output.display());
    }

    Ok(())
}

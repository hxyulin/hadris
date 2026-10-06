use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::WarningKind;
use hadris_io::StdIo;

use crate::common::{Target, read_source};
use crate::cpio::ArchiveFormat;

pub fn create(
    directory: PathBuf,
    target: Target,
    format: ArchiveFormat,
    verbose: bool,
) -> Result<()> {
    let output = &target.output;
    let tree = read_source(&directory)?;

    let (format, name) = match format {
        ArchiveFormat::Newc => (Format::Newc, "newc"),
        ArchiveFormat::Crc => (Format::Crc, "newc+crc"),
        ArchiveFormat::Odc => (Format::Odc, "odc"),
    };
    let options = CpioOptions::default().with_format(format);
    let to_stdout = output.as_os_str() == "-";
    let report = if to_stdout {
        let mut out = StdIo::new(BufWriter::new(io::stdout().lock()));
        let report = hadris_cpio::sync::write(&mut out, &tree, &options)
            .context("Failed to write CPIO archive")?;
        out.into_inner()
            .flush()
            .context("Failed to write CPIO archive")?;
        report
    } else {
        let (file, pending) = target.create()?;
        let mut out = StdIo::new(BufWriter::new(file));
        let report = hadris_cpio::sync::write(&mut out, &tree, &options)
            .context("Failed to write CPIO archive")?;
        let file = out
            .into_inner()
            .into_inner()
            .map_err(|err| err.into_error())
            .context("Failed to write CPIO archive")?;
        pending
            .commit(file)
            .with_context(|| format!("Failed to write output file: {}", output.display()))?;
        report
    };

    for warning in report.warnings() {
        if verbose || !matches!(warning.kind(), WarningKind::Dropped(_)) {
            eprintln!("warning: {warning}");
        }
    }
    if !to_stdout {
        println!("Created {name} archive: {}", output.display());
    }

    Ok(())
}

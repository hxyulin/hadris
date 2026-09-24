use std::io::{self, Write};

use hadris_fs::OpenOptions;
use hadris_fs::sync::DriverExt;

use super::super::args::CatArgs;

use super::{Result, open, view_for};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let mut iso = open(&args.input)?;
    let mut view = view_for(&mut iso, &args.path)?;
    let mut file = view
        .open(&args.path, OpenOptions::read())
        .map_err(|err| format!("File not found: {}: {err}", args.path))?;
    let mut stdout = io::stdout().lock();
    io::copy(&mut file, &mut stdout)?;
    stdout.flush()?;
    Ok(())
}

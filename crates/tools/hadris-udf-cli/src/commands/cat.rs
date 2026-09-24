use std::io::{self, Write};

use hadris_fs::OpenOptions;
use hadris_fs::sync::DriverExt;

use super::super::args::CatArgs;
use super::{Result, open};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let mut file = udf
        .open(&args.path, OpenOptions::read())
        .map_err(|err| format!("File not found: {}: {err}", args.path))?;
    let mut stdout = io::stdout().lock();
    io::copy(&mut file, &mut stdout)?;
    stdout.flush()?;
    Ok(())
}

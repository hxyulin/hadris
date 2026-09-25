use std::io::{self, Write};

use super::super::args::CatArgs;
use super::{Result, copy_file, open};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let mut stdout = io::stdout().lock();
    copy_file(&mut udf, &args.path, &mut stdout)
        .map_err(|err| format!("File not found: {}: {err}", args.path))?;
    stdout.flush()?;
    Ok(())
}

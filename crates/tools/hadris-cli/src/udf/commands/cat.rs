use std::io::{self, Write};

use anyhow::{Context, Result};

use super::super::args::CatArgs;
use super::{copy_file, open};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let mut udf = open(&args.input, &args.locate)?;
    let mut stdout = io::stdout().lock();
    copy_file(&mut udf, &args.path, &mut stdout)
        .with_context(|| format!("File not found: {}", args.path))?;
    stdout.flush()?;
    Ok(())
}

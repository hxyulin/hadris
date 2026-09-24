use std::io::{self, Write};

use hadris_fs::sync::DriverExt;

use super::super::args::CatArgs;
use super::{Result, open};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let bytes = udf.read_to_vec(&args.path)?;
    io::stdout().write_all(&bytes)?;
    Ok(())
}

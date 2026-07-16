use std::fs::File;
use std::io::{self, Write};

use hadris_udf::UdfFs;

use crate::args::CatArgs;

use super::{Result, navigate_to_path};

/// Print file contents to stdout
pub fn cat(args: CatArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfFs::open(file)?;

    let path = args.path.trim_start_matches('/');
    let (parent_path, filename) = if let Some(pos) = path.rfind('/') {
        (&path[..pos], &path[pos + 1..])
    } else {
        ("", path)
    };

    if filename.is_empty() {
        return Err("path must include a file name".into());
    }

    let dir = navigate_to_path(&udf, parent_path)?;
    let entry = dir
        .entries()
        .find(|e| e.is_file() && e.name() == filename)
        .ok_or_else(|| -> Box<dyn std::error::Error> {
            format!("File not found: {}", args.path).into()
        })?;

    let bytes = udf.read_file(entry)?;
    io::stdout().write_all(&bytes)?;
    Ok(())
}

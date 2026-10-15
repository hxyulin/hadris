use anyhow::Result;

use super::super::args::ExtractArgs;
use super::open;
use crate::vfs::{Extraction, Wording};

/// Extract files from a UDF image. The root is merged into the output
/// directory; any other path lands at `<output>/<name>`.
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut udf = open(&args.input, &args.locate)?;
    let from = args.path.as_deref().unwrap_or("/");
    let plan = Extraction::plan(&mut udf, from, &args.output, Wording::Optical)?;
    if args.verbose {
        println!("Extracting {from} to {}", plan.destination.display());
    }
    plan.run(udf)?;
    println!("Extracted {from} to {}", plan.destination.display());
    Ok(())
}

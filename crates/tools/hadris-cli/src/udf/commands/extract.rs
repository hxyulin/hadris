use anyhow::{Context, Result};

use super::super::args::ExtractArgs;
use super::open;
use crate::vfs::{Extraction, NamePolicy};

/// Extract files from a UDF image. The root is merged into the output
/// directory; any other path lands at `<output>/<name>`.
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut udf = open(&args.input, &args.locate)?;
    let from = args.path.as_deref().unwrap_or("/");
    let plan = Extraction::plan(&mut udf, from, &args.output, NamePolicy::Lossy)
        .with_context(|| format!("Not found: {from}"))?;
    if args.verbose {
        println!("Extracting {from} to {}", plan.destination.display());
    }
    plan.run(udf)
        .with_context(|| format!("Failed to extract {from}"))?;
    println!("Extracted {from} to {}", plan.destination.display());
    Ok(())
}

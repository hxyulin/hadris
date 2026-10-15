use anyhow::Result;
use hadris_udf::{UdfId, UdfOptions};

use super::super::args::CreateArgs;
use crate::common::{build_time, print_warnings, read_source};

/// Create a new UDF image
pub fn create(args: CreateArgs) -> Result<()> {
    let output = &args.target.output;
    if args.verbose {
        println!("Creating UDF image from: {}", args.source.display());
        if !args.dry_run {
            println!("Output: {}", output.display());
        }
    }

    let revision = args.revision.0;
    let tree = read_source(&args.source)?;
    let options = UdfOptions::default()
        .with_id(UdfId::Volume, &args.volume_name)
        .with_revision(revision)
        .with_time(build_time()?);

    if args.dry_run {
        let report = hadris_udf::plan(&tree, &options)?;
        println!("Dry run: would create UDF image");
        println!("  Volume name: {}", args.volume_name);
        println!("  UDF revision: {revision}");
        println!(
            "  Size: {} sectors ({} bytes)",
            report.size() / 2048,
            report.size()
        );
        return Ok(());
    }

    let report = args
        .target
        .write_image(0, |dev| Ok(hadris_udf::sync::write(dev, &tree, &options)?))?;
    print_warnings(&report, args.verbose);

    if args.verbose {
        println!(
            "Created UDF image: {} ({} sectors, {} bytes)",
            output.display(),
            report.size() / 2048,
            report.size()
        );
    } else {
        println!("Created: {}", output.display());
    }
    Ok(())
}

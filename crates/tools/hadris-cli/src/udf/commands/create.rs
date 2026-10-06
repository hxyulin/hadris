use hadris_fs::WarningKind;
use hadris_udf::{UdfId, UdfOptions};

use super::super::args::CreateArgs;
use super::Result;
use crate::common::{build_time, read_source};

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

    let (file, pending) = args.target.create()?;
    let mut dev = hadris_storage::host::FileDevice::new(file)
        .map_err(|err| format!("cannot create {}: {err}", output.display()))?;
    let report = hadris_udf::sync::write(&mut dev, &tree, &options)?;
    pending
        .commit(dev.into_inner())
        .map_err(|err| format!("cannot write {}: {err}", output.display()))?;
    for warning in report.warnings() {
        if !matches!(warning.kind(), WarningKind::Dropped(_)) || args.verbose {
            eprintln!("warning: {warning}");
        }
    }

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

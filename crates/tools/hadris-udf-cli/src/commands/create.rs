use hadris_fs::host::{self, OnError, TreeOptions};
use hadris_fs::{Clock, SystemClock, WarningKind};
use hadris_udf::{UdfId, UdfOptions, UdfRevision};

use super::super::args::CreateArgs;
use super::super::output::Output;
use super::Result;

/// Create a new UDF image
pub fn create(args: CreateArgs) -> Result<()> {
    if args.verbose {
        println!("Creating UDF image from: {}", args.source.display());
        if !args.dry_run {
            println!("Output: {}", args.output.display());
        }
    }

    let revision = parse_revision(&args.revision)?;
    let (tree, skipped) = host::read_tree(
        &args.source,
        &TreeOptions::new().with_on_error(OnError::Skip),
    )?;
    for err in skipped {
        eprintln!("warning: skipped {err}");
    }
    let time = host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now());
    let options = UdfOptions::default()
        .with_id(UdfId::Volume, &args.volume_name)
        .with_revision(revision)
        .with_time(time);

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

    let (file, pending) = Output::create(&args.output)
        .map_err(|err| format!("cannot create {}: {err}", args.output.display()))?;
    let mut dev = hadris_storage::host::FileDevice::new(file)
        .map_err(|err| format!("cannot create {}: {err}", args.output.display()))?;
    let report = hadris_udf::sync::write(&mut dev, &tree, &options)?;
    pending
        .commit(dev.into_inner())
        .map_err(|err| format!("cannot write {}: {err}", args.output.display()))?;
    for warning in report.warnings() {
        if !matches!(warning.kind(), WarningKind::Dropped(_)) || args.verbose {
            eprintln!("warning: {warning}");
        }
    }

    if args.verbose {
        println!(
            "Created UDF image: {} ({} sectors, {} bytes)",
            args.output.display(),
            report.size() / 2048,
            report.size()
        );
    } else {
        println!("Created: {}", args.output.display());
    }
    Ok(())
}

/// Parse a UDF revision string like "1.02" into a supported UdfRevision.
fn parse_revision(s: &str) -> Result<UdfRevision> {
    match s {
        "1.02" => Ok(UdfRevision::V1_02),
        "1.50" => Ok(UdfRevision::V1_50),
        "2.00" => Ok(UdfRevision::V2_00),
        "2.01" => Ok(UdfRevision::V2_01),
        "2.50" => Ok(UdfRevision::V2_50),
        "2.60" => Ok(UdfRevision::V2_60),
        _ => Err(format!(
            "invalid UDF revision '{s}': expected 1.02, 1.50, 2.00, 2.01, 2.50, or 2.60"
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_revision_supported() {
        assert_eq!(parse_revision("1.02").unwrap(), UdfRevision::V1_02);
        assert_eq!(parse_revision("2.60").unwrap(), UdfRevision::V2_60);
    }

    #[test]
    fn test_parse_revision_rejects_unsupported() {
        for input in ["9.99", "1.03", "2.5", "abc", ""] {
            assert!(parse_revision(input).is_err(), "should reject {input:?}");
        }
    }
}

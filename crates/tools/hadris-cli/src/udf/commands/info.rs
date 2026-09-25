use super::super::args::InfoArgs;
use super::{Result, entries, open};

/// Display information about a UDF image
pub fn info(args: InfoArgs) -> Result<()> {
    let mut udf = open(&args.input)?;

    println!("UDF Image: {}", args.input.display());
    println!();
    println!("Volume Information:");
    println!(
        "  Volume ID:         {}",
        udf.info().id(hadris_udf::UdfId::Volume)
    );
    println!(
        "  Logical Volume:    {}",
        udf.info().id(hadris_udf::UdfId::LogicalVolume)
    );
    println!("  UDF Revision:      {}", udf.info().revision());
    println!("  Block Size:        {} bytes", udf.info().block_size());
    for (index, partition) in udf.info().partitions().iter().enumerate() {
        println!(
            "  Partition {index}:       number {}, start block {}, {} blocks ({} bytes)",
            partition.number(),
            partition.start(),
            partition.len(),
            u64::from(partition.len()) * u64::from(udf.info().block_size())
        );
    }

    if args.verbose {
        println!();
        println!("Structure:");
        match entries(&mut udf, "/") {
            Ok(root) => println!("  Root directory:    {} entries", root.len()),
            Err(e) => println!("  Root directory:    error ({e})"),
        }
    }
    Ok(())
}

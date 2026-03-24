use std::fs::File;

use hadris_udf::UdfFs;

use crate::args::InfoArgs;

use super::Result;

/// Display information about a UDF image
pub fn info(args: InfoArgs) -> Result<()> {
    let file = File::open(&args.input)?;
    let udf = UdfFs::open(file)?;
    let info = udf.info();

    println!("UDF Image: {}", args.input.display());
    println!();
    println!("Volume Information:");
    println!("  Volume ID:         {}", info.volume_id);
    println!("  UDF Revision:      {}", info.udf_revision);
    println!("  Block Size:        {} bytes", info.block_size);
    println!(
        "  Partition Start:   sector {}",
        info.partition_start
    );
    println!(
        "  Partition Length:  {} sectors ({} bytes)",
        info.partition_length,
        info.partition_length as u64 * info.block_size as u64
    );

    if args.verbose {
        println!();
        println!("Structure:");
        match udf.root_dir() {
            Ok(root) => {
                let count = root.entries().count();
                println!("  Root directory:    {} entries", count);
            }
            Err(e) => {
                println!("  Root directory:    error ({})", e);
            }
        }
    }

    Ok(())
}

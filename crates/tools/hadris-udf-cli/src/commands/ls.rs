use hadris_fs::sync::DriverExt;

use super::super::args::LsArgs;
use super::{Result, entries, join, open, type_char};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    for item in entries(&mut udf, &args.path)? {
        let name = String::from_utf8_lossy(item.name_bytes()).into_owned();
        if args.long {
            let meta = udf.metadata(&join(&args.path, &name))?;
            println!(
                "{}  {:>10}  {}",
                type_char(item.file_type()),
                meta.len(),
                name
            );
        } else if item.file_type().is_dir() {
            println!("{name}/");
        } else {
            println!("{name}");
        }
    }
    Ok(())
}

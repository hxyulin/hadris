use super::super::args::LsArgs;
use super::{Result, entries, open, type_char};

/// List directory contents
pub fn ls(args: LsArgs) -> Result<()> {
    let mut udf = open(&args.input)?;
    let items = entries(&mut udf, &args.path)?;
    if args.all {
        for name in [".", ".."] {
            if args.long {
                println!("d  {:>10}  {name}", "");
            } else {
                println!("{name}/");
            }
        }
    }
    for item in items {
        let name = String::from_utf8_lossy(item.name().as_bytes()).into_owned();
        if args.long {
            let meta = item.metadata();
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

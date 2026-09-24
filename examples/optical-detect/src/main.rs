use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_optical::detect::sync::detect;

fn main() -> Result<()> {
    let image_path = image_path()?;
    let mut image = File::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;

    let Some(formats) = detect(&mut image)
        .with_context(|| format!("failed to inspect {}", image_path.display()))?
    else {
        println!("No supported optical filesystem detected");
        return Ok(());
    };

    println!("ISO 9660: {}", yes_no(formats.has_iso9660()));
    println!(
        "UDF:      {}",
        formats
            .udf()
            .map(|revision| format!("yes ({revision:?})"))
            .unwrap_or_else(|| "no".to_owned())
    );
    println!("Bridge:   {}", yes_no(formats.is_bridge()));

    Ok(())
}

fn image_path() -> Result<PathBuf> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        bail!(
            "usage: {} <optical-image>",
            PathBuf::from(program).display()
        );
    };
    if args.next().is_some() {
        bail!("expected exactly one optical image path");
    }
    Ok(path.into())
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

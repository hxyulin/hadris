//! Imports a host directory, builds a FAT, exFAT, ISO 9660 and UDF image of
//! it, then treats each image as unknown: detects it, opens it read-only
//! with `hadris::host::open`, and extracts it to a host directory through
//! one code path. Every extracted file must match the source.
//!
//! Catalog actions: IO-HOST-01, IO-DETECT-01, IO-DETECT-02, BUILD-TREE-01,
//! HOST-EXTRACT-01, DIR-WALK-01, FILE-READ-01.
//!
//! ```text
//! cargo run -p hadris-example-extract            # in a temporary directory
//! cargo run -p hadris-example-extract -- work/   # keep the images and output
//! ```

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use hadris::ImageFormat;
use hadris::fat::exfat::ExFatOptions;
use hadris::fat::{FatKind, FatOptions};
use hadris::fs::Tree;
use hadris::fs::sync::Volume;
use hadris::host::{FileDevice, TreeOptions};
use hadris::iso::IsoOptions;
use hadris::udf::UdfOptions;

const IMAGE_SIZE: u64 = 16 << 20;

fn main() -> Result<()> {
    let (dir, keep) = match std::env::args_os().nth(1) {
        Some(dir) => (PathBuf::from(dir), true),
        None => (
            std::env::temp_dir().join(format!("hadris-extract-{}", std::process::id())),
            false,
        ),
    };
    let result = run(&dir);
    if !keep {
        std::fs::remove_dir_all(&dir)?;
    }
    result?;
    println!("FAT, exFAT, ISO 9660 and UDF images extracted and compared");
    Ok(())
}

fn run(dir: &Path) -> Result<()> {
    let source = dir.join("source");
    make_source(&source)?;
    let (tree, skipped) = hadris::host::read_tree(&source, &TreeOptions::new())?;
    ensure!(skipped.is_empty());

    let images = [
        ("disk.fat", ImageFormat::Fat(FatKind::Fat16)),
        ("disk.exfat", ImageFormat::ExFat),
        ("disc.iso", ImageFormat::Iso),
        ("disc.udf", ImageFormat::Udf),
    ];
    for (name, format) in images {
        let image = dir.join(name);
        build(&image, format, &tree)?;

        let mut dev = FileDevice::open(&image)?;
        let found = hadris::sync::detect(&mut dev)?;
        ensure!(
            found.first().map(|c| c.format()) == Some(format),
            "{name} detected as {:?}",
            found.first().map(|c| c.format())
        );

        let out = dir.join(format!("{name}.out"));
        let vol = Volume::new(hadris::host::open(&image)?);
        let extracted = hadris::fs::sync::read_tree(&vol, "/")?;
        hadris::host::write_tree(&out, &extracted)?;
        compare(&source, &out).with_context(|| format!("{name} does not match the source"))?;
    }
    Ok(())
}

fn make_source(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root.join("docs"))?;
    std::fs::create_dir_all(root.join("data/nested"))?;
    std::fs::create_dir_all(root.join("empty"))?;
    std::fs::write(root.join("docs/readme.md"), "# Hello\n")?;
    std::fs::write(
        root.join("docs/A long file name.txt"),
        "long names survive\n",
    )?;
    let blob: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(root.join("data/blob.bin"), blob)?;
    std::fs::write(root.join("data/nested/empty.txt"), "")?;
    Ok(())
}

fn build(path: &Path, format: ImageFormat, tree: &Tree) -> Result<()> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    let dev = FileDevice::new(file)?;
    match format {
        ImageFormat::Fat(FatKind::Fat16) => {
            hadris::fat::sync::write(dev, tree, &FatOptions::new().with_size(IMAGE_SIZE))?;
        }
        ImageFormat::ExFat => {
            hadris::fat::exfat::sync::write(dev, tree, &ExFatOptions::new().with_size(IMAGE_SIZE))?;
        }
        ImageFormat::Iso => {
            let options = IsoOptions::default().with_rock_ridge().with_joliet();
            hadris::iso::sync::write(dev, tree, &options)?;
        }
        ImageFormat::Udf => {
            hadris::udf::sync::write(dev, tree, &UdfOptions::default())?;
        }
        other => anyhow::bail!("no writer for {other:?} in this example"),
    }
    Ok(())
}

/// Checks that every directory and file under `expected` exists under
/// `actual` with the same bytes.
fn compare(expected: &Path, actual: &Path) -> Result<()> {
    for entry in std::fs::read_dir(expected)? {
        let entry = entry?;
        let other = actual.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            ensure!(other.is_dir(), "{} is missing", other.display());
            compare(&entry.path(), &other)?;
        } else {
            let want = std::fs::read(entry.path())?;
            let got =
                std::fs::read(&other).with_context(|| format!("{} is missing", other.display()))?;
            ensure!(want == got, "{} differs", other.display());
        }
    }
    Ok(())
}

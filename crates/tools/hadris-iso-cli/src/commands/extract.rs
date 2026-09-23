use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use hadris_fs::FileType;
use hadris_fs::sync::DriverExt;

use super::super::args::ExtractArgs;

use super::{Result, View, join, list_dir, open, preferred_view};

/// Extract files from an ISO image
pub fn extract(args: ExtractArgs) -> Result<()> {
    let mut iso = open(&args.input)?;
    let mut view = preferred_view(&mut iso)?;

    fs::create_dir_all(&args.output)?;
    let start = args.path.as_deref().unwrap_or("/");
    let mut extracted_count = 0;
    extract_dir(
        &mut view,
        start,
        &args.output,
        args.verbose,
        &mut extracted_count,
    )?;

    println!(
        "Extracted {} files to {}",
        extracted_count,
        args.output.display()
    );
    Ok(())
}

fn extract_dir(
    view: &mut View<'_>,
    path: &str,
    output_path: &Path,
    verbose: bool,
    count: &mut usize,
) -> Result<()> {
    for entry in list_dir(view, path)? {
        let source = join(path, &entry.name);
        let entry_path = safe_entry_path(output_path, &entry.name)?;
        match entry.meta.file_type() {
            FileType::Dir => {
                fs::create_dir_all(&entry_path)?;
                if verbose {
                    println!("Creating directory: {}", entry_path.display());
                }
                extract_dir(view, &source, &entry_path, verbose, count)?;
            }
            FileType::File => {
                if verbose {
                    println!(
                        "Extracting: {} ({} bytes)",
                        entry_path.display(),
                        entry.meta.len()
                    );
                }
                fs::write(&entry_path, view.read_to_vec(&source)?)?;
                *count += 1;
            }
            FileType::Symlink => extract_symlink(view, &entry, &entry_path, verbose)?,
            other => eprintln!("Skipping {source}: cannot extract a {other:?}"),
        }
    }

    Ok(())
}

#[cfg(unix)]
fn extract_symlink(
    view: &mut View<'_>,
    entry: &super::Entry,
    entry_path: &Path,
    verbose: bool,
) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let mut target = vec![0u8; 4096];
    let len = view.read_link(entry.node, &mut target)?;
    target.truncate(len);
    if verbose {
        println!(
            "Linking: {} -> {}",
            entry_path.display(),
            String::from_utf8_lossy(&target)
        );
    }
    std::os::unix::fs::symlink(OsStr::from_bytes(&target), entry_path)?;
    Ok(())
}

#[cfg(not(unix))]
fn extract_symlink(
    _view: &mut View<'_>,
    _entry: &super::Entry,
    entry_path: &Path,
    _verbose: bool,
) -> Result<()> {
    eprintln!(
        "Skipping {}: symlinks are not supported here",
        entry_path.display()
    );
    Ok(())
}

fn safe_entry_path(output_path: &Path, name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if component == OsStr::new(name) => {
            Ok(output_path.join(path))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsafe filename in ISO image: {name:?}"),
        )
        .into()),
    }
}

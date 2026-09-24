use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use hadris_cpio::{CpioArchiveReader, FileType};

pub fn extract(archive: PathBuf, output: PathBuf) -> Result<()> {
    let file = File::open(&archive)
        .with_context(|| format!("Failed to open archive: {}", archive.display()))?;
    let mut reader = CpioArchiveReader::new(BufReader::new(file));

    fs::create_dir_all(&output)
        .with_context(|| format!("Failed to create output directory: {}", output.display()))?;

    let mut count: u64 = 0;

    while let Some(entry) = reader.next_entry_alloc().context("Failed to read entry")? {
        let name = entry.name_str().unwrap_or("<invalid utf-8>").to_string();
        let header = entry.header().clone();
        let ft = entry.file_type();

        let Some(dest) = safe_dest(&output, &name) else {
            eprintln!("warning: skipping {name} (unsafe path)");
            reader.skip_entry_data_owned(&entry)?;
            continue;
        };
        if matches!(ft, FileType::Directory | FileType::Regular)
            && dest
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        {
            fs::remove_file(&dest)
                .with_context(|| format!("Failed to replace symlink: {}", dest.display()))?;
        }

        match ft {
            FileType::Directory => {
                fs::create_dir_all(&dest)
                    .with_context(|| format!("Failed to create directory: {}", dest.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = fs::Permissions::from_mode(header.permissions());
                    fs::set_permissions(&dest, perms).ok();
                }
                reader.skip_entry_data_owned(&entry)?;
            }
            FileType::Regular => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let data = reader
                    .read_entry_data_alloc(&entry)
                    .context("Failed to read file data")?;
                fs::write(&dest, &data)
                    .with_context(|| format!("Failed to write file: {}", dest.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = fs::Permissions::from_mode(header.permissions());
                    fs::set_permissions(&dest, perms).ok();
                }
            }
            FileType::Symlink => {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let target_data = reader
                    .read_entry_data_alloc(&entry)
                    .context("Failed to read symlink target")?;
                let target = std::str::from_utf8(&target_data)
                    .context("Symlink target is not valid UTF-8")?;
                // Remove existing file/symlink if present
                if dest.exists() || dest.symlink_metadata().is_ok() {
                    fs::remove_file(&dest).ok();
                }
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(target, &dest)
                        .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
                }
                #[cfg(not(unix))]
                {
                    std::os::windows::fs::symlink_file(target, &dest)
                        .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
                }
            }
            _ => {
                eprintln!("warning: skipping {name} ({ft})");
                reader.skip_entry_data_owned(&entry)?;
            }
        }

        count += 1;
    }

    println!("Extracted {} entries to {}", count, output.display());

    Ok(())
}

/// Resolves an archive entry name under `output`, rejecting names that are
/// absolute, contain `..`, or pass through an already-extracted symlink.
fn safe_dest(output: &Path, name: &str) -> Option<PathBuf> {
    let mut dest = output.to_path_buf();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => {
                if dest != output
                    && dest
                        .symlink_metadata()
                        .is_ok_and(|m| m.file_type().is_symlink())
                {
                    return None;
                }
                dest.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(dest)
}

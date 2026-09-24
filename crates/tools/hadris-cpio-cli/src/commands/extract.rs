use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_fs::FileType;
use hadris_io::sync::Read;

use super::open_reader;

/// Where `name` goes below `output`, or `None` for a name that is the
/// output directory itself, such as `.` or `./`. Leading `/` and `.`
/// components are dropped; `..`, drive prefixes and paths through an
/// existing symlink are refused, so an archive cannot write outside
/// `output`.
fn destination(output: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut dest = output.to_path_buf();
    let mut any = false;
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => {
                if any && dest.symlink_metadata().is_ok_and(|meta| meta.is_symlink()) {
                    bail!("refusing to extract through a symlink: {name}");
                }
                dest.push(part);
                any = true;
            }
            Component::RootDir | Component::CurDir => {}
            _ => bail!("refusing to extract outside the output directory: {name}"),
        }
    }
    Ok(any.then_some(dest))
}

fn replace(dest: &Path) {
    if dest
        .symlink_metadata()
        .is_ok_and(|meta| !meta.is_dir() || meta.is_symlink())
    {
        fs::remove_file(dest).ok();
    }
}

#[cfg(unix)]
fn set_mode(dest: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dest, fs::Permissions::from_mode(mode & 0o7777)).ok();
}

#[cfg(not(unix))]
fn set_mode(_dest: &Path, _mode: u32) {}

/// Names of a hard link group: the path holding the data, and names seen
/// before the data.
#[derive(Default)]
struct Links {
    data: Option<PathBuf>,
    waiting: Vec<PathBuf>,
}

pub fn extract(archive: PathBuf, output: PathBuf) -> Result<()> {
    let mut reader = open_reader(&archive)?;
    fs::create_dir_all(&output)
        .with_context(|| format!("Failed to create output directory: {}", output.display()))?;

    let mut count: u64 = 0;
    let mut links: HashMap<u64, Links> = HashMap::new();
    let mut buf = vec![0u8; 64 * 1024];

    while let Some(mut entry) = reader.next_entry().context("Failed to read entry")? {
        let name = entry
            .name_str()
            .context("Entry name is not UTF-8")?
            .to_string();
        let Some(dest) = destination(&output, &name)? else {
            if entry.file_type() != FileType::Dir {
                bail!("refusing to extract an entry without a name: {name}");
            }
            count += 1;
            continue;
        };
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        match entry.file_type() {
            FileType::Dir => {
                replace(&dest);
                fs::create_dir_all(&dest)
                    .with_context(|| format!("Failed to create directory: {}", dest.display()))?;
                set_mode(&dest, entry.mode());
            }
            FileType::File => {
                let group = (entry.nlink() > 1).then(|| links.entry(entry.ino()).or_default());
                replace(&dest);
                match group {
                    Some(group) if entry.len() == 0 => {
                        if let Some(data) = &group.data {
                            fs::hard_link(data, &dest)
                                .with_context(|| format!("Failed to link {}", dest.display()))?;
                        } else {
                            File::create(&dest)?;
                            group.waiting.push(dest.clone());
                        }
                    }
                    group => {
                        let mut file = File::create(&dest)
                            .with_context(|| format!("Failed to write file: {}", dest.display()))?;
                        loop {
                            let read = entry.read(&mut buf).context("Failed to read file data")?;
                            if read == 0 {
                                break;
                            }
                            file.write_all(&buf[..read])?;
                        }
                        if let Some(group) = group {
                            for waiting in group.waiting.drain(..) {
                                fs::remove_file(&waiting).ok();
                                fs::hard_link(&dest, &waiting).with_context(|| {
                                    format!("Failed to link {}", waiting.display())
                                })?;
                            }
                            group.data = Some(dest.clone());
                        }
                    }
                }
                set_mode(&dest, entry.mode());
            }
            FileType::Symlink => {
                let mut target = Vec::new();
                loop {
                    let read = entry
                        .read(&mut buf)
                        .context("Failed to read symlink target")?;
                    if read == 0 {
                        break;
                    }
                    target.extend_from_slice(&buf[..read]);
                }
                let target =
                    String::from_utf8(target).context("Symlink target is not valid UTF-8")?;
                replace(&dest);
                #[cfg(unix)]
                std::os::unix::fs::symlink(&target, &dest)
                    .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
                #[cfg(not(unix))]
                std::os::windows::fs::symlink_file(&target, &dest)
                    .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
            }
            other => {
                eprintln!("warning: skipping {name} ({other:?})");
            }
        }
        count += 1;
    }

    println!("Extracted {} entries to {}", count, output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_inside_the_output() {
        let out = Path::new("/out");
        let at = |name| destination(out, name).unwrap().unwrap();
        assert_eq!(at("a/b"), Path::new("/out/a/b"));
        assert_eq!(at("/etc/x"), Path::new("/out/etc/x"));
        assert_eq!(at("./a"), Path::new("/out/a"));
        assert!(destination(out, "../x").is_err());
        assert!(destination(out, "a/../../x").is_err());
        assert_eq!(destination(out, ".").unwrap(), None);
        assert_eq!(destination(out, "./").unwrap(), None);
    }
}

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_fs::FileType;
use hadris_io::sync::Read;

use hadris_cpio::sync::Entry;

use super::{Input, open_reader};

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

/// Clears the way for an entry at `dest`. A file or symlink that an earlier
/// entry of this archive created is removed; one that was there before the
/// extraction fails, as `hadris_fs::host::write_tree` does. Directories are
/// merged.
fn replace(dest: &Path, created: &HashSet<PathBuf>) -> Result<()> {
    if dest
        .symlink_metadata()
        .is_ok_and(|meta| !meta.is_dir() || meta.is_symlink())
    {
        if !created.contains(dest) {
            bail!("refusing to replace an existing file: {}", dest.display());
        }
        fs::remove_file(dest).ok();
    }
    Ok(())
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

/// Where an entry named `name` lands when extracting `select`, the
/// components of the requested path: `None` outside it, and otherwise the
/// name below the last selected component, as `--path` works in the other
/// formats.
fn relocate(select: &[&str], name: &str) -> Option<String> {
    let Some(last) = select.last() else {
        return Some(name.to_owned());
    };
    let parts: Vec<&str> = name
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if !parts.starts_with(select) {
        return None;
    }
    let mut out = (*last).to_owned();
    for part in &parts[select.len()..] {
        out.push('/');
        out.push_str(part);
    }
    Some(out)
}

fn copy_data(entry: &mut Entry<'_, Input>, dest: &Path, buf: &mut [u8]) -> Result<()> {
    let mut file =
        File::create(dest).with_context(|| format!("Failed to write file: {}", dest.display()))?;
    loop {
        let read = entry.read(buf).context("Failed to read file data")?;
        if read == 0 {
            return Ok(());
        }
        file.write_all(&buf[..read])?;
    }
}

fn link_waiting(group: &mut Links, data: &Path) -> Result<()> {
    for waiting in group.waiting.drain(..) {
        fs::remove_file(&waiting).ok();
        fs::hard_link(data, &waiting)
            .with_context(|| format!("Failed to link {}", waiting.display()))?;
    }
    group.data = Some(data.to_path_buf());
    Ok(())
}

pub fn extract(archive: PathBuf, output: PathBuf, path: Option<&str>) -> Result<()> {
    let select: Vec<&str> = path
        .unwrap_or("/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if select.contains(&"..") {
        bail!(
            "refusing to extract a path with `..`: {}",
            path.unwrap_or("/")
        );
    }
    let mut reader = open_reader(&archive)?;
    fs::create_dir_all(&output)
        .with_context(|| format!("Failed to create output directory: {}", output.display()))?;

    let mut count: u64 = 0;
    let mut links: HashMap<u64, Links> = HashMap::new();
    let mut created = HashSet::new();
    let mut buf = vec![0u8; 64 * 1024];

    while let Some(mut entry) = reader.next_entry().context("Failed to read entry")? {
        let stored = entry
            .name_str()
            .context("Entry name is not UTF-8")?
            .to_string();
        let Some(name) = relocate(&select, &stored) else {
            // Hard-link data outside the selection still fills the selected
            // names of its group.
            if entry.file_type() == FileType::File && entry.nlink() > 1 && entry.len() > 0 {
                if let Some(group) = links.get_mut(&entry.ino()) {
                    if group.data.is_none() && !group.waiting.is_empty() {
                        let data = group.waiting.remove(0);
                        copy_data(&mut entry, &data, &mut buf)?;
                        link_waiting(group, &data)?;
                    }
                }
            }
            continue;
        };
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
                replace(&dest, &created)?;
                fs::create_dir_all(&dest)
                    .with_context(|| format!("Failed to create directory: {}", dest.display()))?;
                set_mode(&dest, entry.mode());
            }
            FileType::File => {
                let group = (entry.nlink() > 1).then(|| links.entry(entry.ino()).or_default());
                replace(&dest, &created)?;
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
                        copy_data(&mut entry, &dest, &mut buf)?;
                        if let Some(group) = group {
                            link_waiting(group, &dest)?;
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
                replace(&dest, &created)?;
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
        created.insert(dest);
        count += 1;
    }

    if count == 0 && !select.is_empty() {
        bail!("Not found in archive: {}", path.unwrap_or("/"));
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

    #[test]
    fn a_selected_path_lands_below_its_last_component() {
        let select = ["usr", "lib"];
        assert_eq!(relocate(&select, "./usr/lib").as_deref(), Some("lib"));
        assert_eq!(
            relocate(&select, "/usr/lib/a.so").as_deref(),
            Some("lib/a.so")
        );
        assert_eq!(relocate(&select, "usr/libexec"), None);
        assert_eq!(relocate(&select, "usr"), None);
        assert_eq!(relocate(&[], "a/b").as_deref(), Some("a/b"));
    }
}

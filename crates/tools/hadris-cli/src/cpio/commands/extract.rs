use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Seek, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_fs::FileType;
use hadris_io::sync::Read;

use hadris_cpio::sync::Entry;

use super::{GroupKey, Input, for_each_entry, link_key, open_reader};

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
        fs::remove_file(dest).with_context(|| format!("Failed to replace {}", dest.display()))?;
    }
    Ok(())
}

/// Creates the regular file `dest`, failing if anything, a symlink
/// included, is already there.
fn create_file(dest: &Path) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(dest)
        .with_context(|| format!("Failed to write file: {}", dest.display()))
}

#[cfg(unix)]
fn set_mode(file: &File, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode)).ok();
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: u32) {}

#[cfg(unix)]
fn set_dir_mode(dir: &Path, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(dir) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(dir)
    {
        set_mode(&dir, mode);
    }
}

#[cfg(not(unix))]
fn set_dir_mode(_dir: &Path, _mode: u32) {}

/// Creates `dir` and its missing parents below `output`, recording the
/// directories this run creates in `made`. An existing directory is
/// accepted; anything else in the way fails.
fn make_dirs(output: &Path, dir: &Path, made: &mut HashSet<PathBuf>) -> Result<()> {
    if dir == output {
        return Ok(());
    }
    match dir.symlink_metadata() {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(_) => bail!("refusing to replace an existing file: {}", dir.display()),
        Err(_) => {}
    }
    if let Some(parent) = dir.parent() {
        make_dirs(output, parent, made)?;
    }
    fs::create_dir(dir)
        .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    made.insert(dir.to_path_buf());
    Ok(())
}

/// Fails unless `path` is a regular file, not a symlink, that this run
/// created, so a hard link never reaches a file outside the extraction.
fn ours(path: &Path, created: &HashSet<PathBuf>) -> Result<()> {
    let regular = path.symlink_metadata().is_ok_and(|meta| meta.is_file());
    if !regular || !created.contains(path) {
        bail!(
            "refusing to hard link a file this extraction did not create: {}",
            path.display()
        );
    }
    Ok(())
}

/// Makes `name` another name of `data`, replacing the file at `name`.
fn relink(data: &Path, name: &Path, created: &HashSet<PathBuf>) -> Result<()> {
    ours(data, created)?;
    ours(name, created)?;
    fs::remove_file(name).with_context(|| format!("Failed to link {}", name.display()))?;
    fs::hard_link(data, name).with_context(|| format!("Failed to link {}", name.display()))
}

/// The extracted names of a hard link group, in archive order. Once
/// `linked`, they are all names of the file holding the group's data;
/// before, each is an empty file waiting for it.
#[derive(Default)]
struct Group {
    names: Vec<PathBuf>,
    linked: bool,
    data: Option<File>,
}

/// The hard link groups, with the group each name belongs to, so a later
/// entry of that name takes it out of its group, as
/// `hadris_cpio::sync::read_tree` does.
#[derive(Default)]
struct Links {
    groups: HashMap<GroupKey, Group>,
    owners: HashMap<PathBuf, GroupKey>,
}

impl Links {
    fn release(&mut self, path: &Path) {
        if let Some(key) = self.owners.remove(path)
            && let Some(group) = self.groups.get_mut(&key)
        {
            group.names.retain(|name| name != path);
            if group.names.is_empty() {
                group.linked = false;
            }
        }
    }
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

/// The state of one extraction.
struct Extractor {
    output: PathBuf,
    /// The permission bits kept from the archive.
    mask: u32,
    /// Files and symlinks this run created.
    created: HashSet<PathBuf>,
    /// Directories this run created.
    made: HashSet<PathBuf>,
    /// Modes of directory entries, applied once the archive ends.
    dir_modes: HashMap<PathBuf, u32>,
    links: Links,
    buf: Vec<u8>,
}

impl Extractor {
    /// Writes the data of `entry` to a new file at `dest`.
    fn write_file(&mut self, entry: &mut Entry<'_, Input>, dest: &Path) -> Result<()> {
        let mut file = create_file(dest)?;
        self.created.insert(dest.to_path_buf());
        let mut retained = if entry.nlink() > 1 && entry.len() > 0 {
            Some(tempfile::tempfile().context("Failed to retain hard link data")?)
        } else {
            None
        };
        loop {
            let read = entry
                .read(&mut self.buf)
                .context("Failed to read file data")?;
            if read == 0 {
                break;
            }
            if let Some(data) = &mut retained {
                data.write_all(&self.buf[..read])?;
            }
            file.write_all(&self.buf[..read])
                .with_context(|| format!("Failed to write file: {}", dest.display()))?;
        }
        if let Some(data) = retained {
            self.links.groups.entry(link_key(entry)).or_default().data = Some(data);
        }
        if entry.nlink() > 1 && entry.len() == 0 {
            let group = self.links.groups.entry(link_key(entry)).or_default();
            if let Some(data) = &mut group.data {
                data.rewind()?;
                std::io::copy(data, &mut file)?;
                group.linked = true;
            }
        }
        set_mode(&file, entry.mode() & self.mask);
        Ok(())
    }

    /// Writes the data of `entry` to `dest`, which is clear, and makes
    /// every name of the group `key` a link to it.
    fn fill_group(
        &mut self,
        entry: &mut Entry<'_, Input>,
        key: GroupKey,
        dest: &Path,
    ) -> Result<()> {
        self.write_file(entry, dest)?;
        let group = self.links.groups.entry(key).or_default();
        for name in &group.names {
            if name != dest {
                relink(dest, name, &self.created)?;
            }
        }
        group.linked = true;
        Ok(())
    }

    /// Hard link data whose own name is outside the selection still fills
    /// the selected names of its group.
    fn unselected(&mut self, entry: &mut Entry<'_, Input>) -> Result<()> {
        if entry.file_type() != FileType::File || entry.nlink() < 2 || entry.is_empty() {
            return Ok(());
        }
        let key = link_key(entry);
        let Some(first) = self
            .links
            .groups
            .get(&key)
            .and_then(|group| group.names.first().cloned())
        else {
            let mut data = tempfile::tempfile().context("Failed to retain hard link data")?;
            loop {
                let n = entry
                    .read(&mut self.buf)
                    .context("Failed to read file data")?;
                if n == 0 {
                    break;
                }
                data.write_all(&self.buf[..n])?;
            }
            self.links.groups.entry(key).or_default().data = Some(data);
            return Ok(());
        };
        ours(&first, &self.created)?;
        fs::remove_file(&first)
            .with_context(|| format!("Failed to replace {}", first.display()))?;
        self.fill_group(entry, key, &first)
    }

    fn file(&mut self, entry: &mut Entry<'_, Input>, dest: &Path) -> Result<()> {
        if entry.nlink() < 2 {
            return self.write_file(entry, dest);
        }
        let key = link_key(entry);
        if !entry.is_empty() {
            self.fill_group(entry, key, dest)?;
        } else {
            let group = self.links.groups.entry(key).or_default();
            match group.names.first() {
                Some(data) if group.linked => {
                    ours(data, &self.created)?;
                    fs::hard_link(data, dest)
                        .with_context(|| format!("Failed to link {}", dest.display()))?;
                    self.created.insert(dest.to_path_buf());
                }
                _ => self.write_file(entry, dest)?,
            }
        }
        self.links
            .groups
            .entry(key)
            .or_default()
            .names
            .push(dest.to_path_buf());
        self.links.owners.insert(dest.to_path_buf(), key);
        Ok(())
    }

    fn symlink(&mut self, entry: &mut Entry<'_, Input>, dest: &Path) -> Result<()> {
        let mut target = Vec::new();
        loop {
            let read = entry
                .read(&mut self.buf)
                .context("Failed to read symlink target")?;
            if read == 0 {
                break;
            }
            target.extend_from_slice(&self.buf[..read]);
        }
        let target = String::from_utf8(target).context("Symlink target is not valid UTF-8")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, dest)
            .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
        #[cfg(not(unix))]
        std::os::windows::fs::symlink_file(&target, dest)
            .with_context(|| format!("Failed to create symlink: {}", dest.display()))?;
        self.created.insert(dest.to_path_buf());
        Ok(())
    }

    /// Applies the directory modes, deepest first so a read-only directory
    /// does not block its children, and only to directories this run
    /// created.
    fn finish(self) {
        let mut dirs: Vec<(PathBuf, u32)> = self
            .dir_modes
            .into_iter()
            .filter(|(dir, _)| self.made.contains(dir))
            .collect();
        dirs.sort_by_key(|(dir, _)| std::cmp::Reverse(dir.components().count()));
        for (dir, mode) in dirs {
            set_dir_mode(&dir, mode & self.mask);
        }
    }
}

pub fn extract(
    archive: PathBuf,
    output: PathBuf,
    path: Option<&str>,
    preserve_special_bits: bool,
) -> Result<()> {
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
    let mut x = Extractor {
        output: output.clone(),
        mask: if preserve_special_bits { 0o7777 } else { 0o777 },
        created: HashSet::new(),
        made: HashSet::new(),
        dir_modes: HashMap::new(),
        links: Links::default(),
        buf: vec![0u8; 64 * 1024],
    };

    let mut current = 1;
    for_each_entry(&mut reader, |segment, entry| {
        if segment != current {
            current = segment;
            x.links = Links::default();
        }
        let stored = entry
            .path_str()
            .context("Entry name is not UTF-8")?
            .to_string();
        let Some(name) = relocate(&select, &stored) else {
            return x.unselected(entry);
        };
        let Some(dest) = destination(&output, &name)? else {
            if entry.file_type() != FileType::Dir {
                bail!("refusing to extract an entry without a name: {name}");
            }
            count += 1;
            return Ok(());
        };
        let file_type = entry.file_type();
        if !matches!(
            file_type,
            FileType::Dir | FileType::File | FileType::Symlink
        ) {
            eprintln!("warning: skipping {name} ({file_type:?})");
            count += 1;
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            make_dirs(&x.output, parent, &mut x.made)?;
        }
        x.links.release(&dest);
        replace(&dest, &x.created)?;
        match file_type {
            FileType::Dir => {
                make_dirs(&x.output, &dest, &mut x.made)?;
                x.dir_modes.insert(dest, entry.mode());
            }
            FileType::File => x.file(entry, &dest)?,
            _ => x.symlink(entry, &dest)?,
        }
        count += 1;
        Ok(())
    })?;
    x.finish();

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

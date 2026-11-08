//! Commands and helpers over any [`FileSystem`] driver.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_fs::host;
use hadris_fs::sync::{FileSystem, Volume, read_tree};
use hadris_fs::{Attributes, DirCursor, DirEntry, Metadata, NodeId, OpenMode, Resolve};

/// Resolves `path` and pins the node.
pub fn resolve<F: FileSystem>(fs: &mut F, path: &str) -> Result<NodeId> {
    fs.resolve(path.as_bytes(), Resolve::Lexical)
        .with_context(|| format!("Failed to open: {path}"))
}

/// The entries of the directory `dir`, in directory order.
pub fn entries<F: FileSystem>(fs: &mut F, dir: NodeId) -> Result<Vec<DirEntry>> {
    let mut items = Vec::new();
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        items.push(entry);
    }
    Ok(items)
}

/// The entries of the directory at `path` with their metadata, in directory
/// order. Fails on names that are not UTF-8.
pub fn list_dir<F: FileSystem>(fs: &mut F, path: &str) -> Result<Vec<(String, Metadata, NodeId)>> {
    let dir = resolve(fs, path)?;
    let mut entries = Vec::new();
    let mut cursor = DirCursor::START;
    let listed = loop {
        match fs.readdir(dir, cursor) {
            Ok(Some(entry)) => {
                cursor = entry.next_cursor();
                match std::str::from_utf8(entry.name().as_bytes()) {
                    Ok(name) => entries.push((name.to_string(), *entry.metadata(), entry.node())),
                    Err(_) => {
                        break Err(anyhow::anyhow!("Directory entry name is not valid UTF-8"));
                    }
                }
            }
            Ok(None) => break Ok(entries),
            Err(err) => break Err(err).context("Failed to read directory entry"),
        }
    };
    fs.forget(dir, 1);
    listed
}

fn attribute_flags(attrs: Attributes) -> String {
    [
        (Attributes::READ_ONLY, 'r'),
        (Attributes::HIDDEN, 'h'),
        (Attributes::SYSTEM, 's'),
        (Attributes::ARCHIVE, 'a'),
    ]
    .iter()
    .map(|&(flag, c)| if attrs.contains(flag) { c } else { '-' })
    .collect()
}

/// Lists the directory at `path`; `long` adds the type, DOS attributes and
/// size.
pub fn ls<F: FileSystem>(fs: &mut F, path: &str, long: bool) -> Result<()> {
    for (name, meta, _) in list_dir(fs, path)? {
        let is_dir = meta.file_type().is_dir();
        if long {
            println!(
                "{}{} {:>10}  {}",
                if is_dir { 'd' } else { '-' },
                attribute_flags(meta.attributes()),
                if is_dir {
                    "<DIR>".to_string()
                } else {
                    meta.len().to_string()
                },
                name
            );
        } else if is_dir {
            println!("{name}/");
        } else {
            println!("{name}");
        }
    }
    Ok(())
}

/// Copies the file at `path` to stdout.
pub fn cat<F: FileSystem>(fs: &mut F, path: &str) -> Result<()> {
    let node = resolve(fs, path)?;
    let copied = copy_to_stdout(fs, node);
    fs.forget(node, 1);
    copied.with_context(|| format!("Failed to read file: {path}"))
}

fn copy_to_stdout<F: FileSystem>(fs: &mut F, node: NodeId) -> Result<()> {
    fs.open(node, OpenMode::Read)?;
    let mut stdout = std::io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    let mut offset = 0u64;
    let copied = loop {
        match fs.read(node, offset, &mut buf) {
            Ok(0) => break stdout.flush().context("Failed to write file to stdout"),
            Ok(n) => {
                if let Err(err) = stdout.write_all(&buf[..n]) {
                    break Err(err).context("Failed to write file to stdout");
                }
                offset += n as u64;
            }
            Err(err) => break Err(err.into()),
        }
    };
    let closed = fs.close(node);
    copied?;
    closed?;
    Ok(())
}

/// Extracts `path` (the root when `None`) below `output`. The root is merged
/// into `output`; anything else lands at `output/<stored name>`.
pub fn extract<F: FileSystem + Send + 'static>(
    mut fs: F,
    output: &Path,
    path: Option<&str>,
) -> Result<()> {
    let from = path.unwrap_or("/");
    let plan = Extraction::plan(&mut fs, from, output, NamePolicy::StrictUtf8)
        .with_context(|| format!("Failed to open: {from}"))?;
    plan.run(fs)
        .with_context(|| format!("Failed to extract {from} to {}", output.display()))
}

/// How the selected entry's name is converted to a host path component.
#[derive(Debug, Clone, Copy)]
pub enum NamePolicy {
    StrictUtf8,
    Lossy,
}

impl NamePolicy {
    fn decode(self, name: &[u8]) -> Result<String> {
        Ok(match self {
            Self::StrictUtf8 => std::str::from_utf8(name)
                .context("Directory entry name is not valid UTF-8")?
                .to_string(),
            Self::Lossy => String::from_utf8_lossy(name).into_owned(),
        })
    }
}

/// A path to extract and where it lands.
#[derive(Debug)]
pub struct Extraction {
    from: String,
    /// The host path the extracted path lands at.
    pub destination: PathBuf,
    target: PathBuf,
}

impl Extraction {
    /// Plans extracting `from` below `output`. The root is merged into
    /// `output`; anything else lands at `output/<stored name>`.
    pub fn plan<F: FileSystem>(
        fs: &mut F,
        from: &str,
        output: &Path,
        names: NamePolicy,
    ) -> Result<Self> {
        let node = fs.resolve(from.as_bytes(), Resolve::Lexical)?;
        let selected = (|| {
            let name = stored_name(fs, from, node, names)?;
            let is_dir = fs.stat(node)?.file_type().is_dir();
            Ok::<_, anyhow::Error>((name, is_dir))
        })();
        fs.forget(node, 1);
        let (name, is_dir) = selected?;
        let (destination, target) = match name {
            None => (output.to_path_buf(), output.to_path_buf()),
            Some(name) if is_dir => (output.join(&name), output.join(name)),
            Some(name) => (output.join(name), output.to_path_buf()),
        };
        Ok(Self {
            from: from.to_string(),
            destination,
            target,
        })
    }

    /// Extracts the planned path of `fs` and prints the extraction warnings.
    pub fn run<F: FileSystem + Send + 'static>(&self, fs: F) -> Result<()> {
        let (from, target) = (self.from.as_str(), &self.target);
        let vol = Volume::new(fs);
        let tree = read_tree(&vol, from).with_context(|| format!("Failed to read {from}"))?;
        fs::create_dir_all(target)
            .with_context(|| format!("Failed to create output directory: {}", target.display()))?;
        let report = host::write_tree(target, &tree)?;
        for warning in report.warnings() {
            eprintln!("warning: {warning}");
        }
        Ok(())
    }
}

/// The name the pinned `node` at `path` is stored under in its directory, or
/// `None` for the root. Fails on names that are not one plain host path
/// component.
fn stored_name<F: FileSystem>(
    fs: &mut F,
    path: &str,
    node: NodeId,
    names: NamePolicy,
) -> Result<Option<String>> {
    if node == fs.root() {
        return Ok(None);
    }
    let name = names.decode(&find_name(fs, path, node)?)?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        bail!("Refusing to extract {path}: its stored name {name:?} is not a plain file name");
    }
    Ok(Some(name))
}

/// Scans the parent of `path` for the entry listed with id `node`.
fn find_name<F: FileSystem>(fs: &mut F, path: &str, node: NodeId) -> Result<Vec<u8>> {
    let parent_path = format!("{path}/..");
    let parent = fs.resolve(parent_path.as_bytes(), Resolve::Lexical)?;
    let mut cursor = DirCursor::START;
    let found = loop {
        match fs.readdir(parent, cursor) {
            Ok(Some(entry)) if entry.node() == node => break Ok(entry),
            Ok(Some(entry)) => cursor = entry.next_cursor(),
            Ok(None) => break Err(anyhow::anyhow!("{path} is not listed in its directory")),
            Err(err) => break Err(err.into()),
        }
    };
    fs.forget(parent, 1);
    let entry = found?;
    Ok(entry.name().as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::NamePolicy;

    #[test]
    fn extraction_name_policies_preserve_valid_names_and_handle_invalid_utf8() {
        for policy in [NamePolicy::StrictUtf8, NamePolicy::Lossy] {
            assert_eq!(policy.decode("café.txt".as_bytes()).unwrap(), "café.txt");
        }
        assert!(NamePolicy::StrictUtf8.decode(b"bad\xff.txt").is_err());
        assert_eq!(
            NamePolicy::Lossy.decode(b"bad\xff.txt").unwrap(),
            "bad\u{fffd}.txt"
        );
    }
}

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
    Extraction::plan(&mut fs, path.unwrap_or("/"), output, Wording::Fat)?.run(fs)
}

/// How an extract command words its errors and decodes names: FAT and the
/// generic commands refuse names that are not UTF-8, ISO 9660 and UDF
/// decode them lossily.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wording {
    Fat,
    Optical,
}

/// A path to extract and where it lands.
#[derive(Debug)]
pub struct Extraction {
    from: String,
    /// The host path the extracted path lands at.
    pub destination: PathBuf,
    target: PathBuf,
    wording: Wording,
}

impl Extraction {
    /// Plans extracting `from` below `output`. The root is merged into
    /// `output`; anything else lands at `output/<stored name>`.
    pub fn plan<F: FileSystem>(
        fs: &mut F,
        from: &str,
        output: &Path,
        wording: Wording,
    ) -> Result<Self> {
        let node = match wording {
            Wording::Fat => resolve(fs, from)?,
            Wording::Optical => fs
                .resolve(from.as_bytes(), Resolve::Lexical)
                .with_context(|| format!("Not found: {from}"))?,
        };
        let name = stored_name(fs, from, node, wording);
        let is_dir = name
            .is_ok()
            .then(|| fs.stat(node).map(|meta| meta.file_type().is_dir()));
        fs.forget(node, 1);
        let name = name?;
        let is_dir = is_dir.unwrap_or(Ok(false))?;
        let (destination, target) = match name {
            None => (output.to_path_buf(), output.to_path_buf()),
            Some(name) if is_dir => (output.join(&name), output.join(name)),
            Some(name) => (output.join(name), output.to_path_buf()),
        };
        Ok(Self {
            from: from.to_string(),
            destination,
            target,
            wording,
        })
    }

    /// Extracts the planned path of `fs` and prints the extraction warnings.
    pub fn run<F: FileSystem + Send + 'static>(&self, fs: F) -> Result<()> {
        let (from, target) = (self.from.as_str(), &self.target);
        let vol = Volume::new(fs);
        let tree = match self.wording {
            Wording::Fat => {
                fs::create_dir_all(target).with_context(|| {
                    format!("Failed to create output directory: {}", target.display())
                })?;
                read_tree(&vol, from).with_context(|| format!("Failed to read {from}"))?
            }
            Wording::Optical => {
                let tree =
                    read_tree(&vol, from).with_context(|| format!("Failed to extract {from}"))?;
                fs::create_dir_all(target)?;
                tree
            }
        };
        let report = host::write_tree(target, &tree).with_context(|| match self.wording {
            Wording::Fat => format!("Failed to extract {from} to {}", target.display()),
            Wording::Optical => format!("Failed to extract {from}"),
        })?;
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
    wording: Wording,
) -> Result<Option<String>> {
    if node == fs.root() {
        return Ok(None);
    }
    let name = find_name(fs, path, node, wording)?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        let what = match wording {
            Wording::Fat => "stored name",
            Wording::Optical => "name",
        };
        bail!("Refusing to extract {path}: its {what} {name:?} is not a plain file name");
    }
    Ok(Some(name))
}

/// Scans the parent of `path` for the entry listed with id `node`.
fn find_name<F: FileSystem>(
    fs: &mut F,
    path: &str,
    node: NodeId,
    wording: Wording,
) -> Result<String> {
    let parent_path = format!("{path}/..");
    let parent = match wording {
        Wording::Fat => resolve(fs, &parent_path)?,
        Wording::Optical => fs.resolve(parent_path.as_bytes(), Resolve::Lexical)?,
    };
    let mut cursor = DirCursor::START;
    let found = loop {
        match fs.readdir(parent, cursor) {
            Ok(Some(entry)) if entry.node() == node => break Ok(entry),
            Ok(Some(entry)) => cursor = entry.next_cursor(),
            Ok(None) => break Err(anyhow::anyhow!("{path} is not listed in its directory")),
            Err(err) if wording == Wording::Fat => {
                break Err(err).context("Failed to read directory entry");
            }
            Err(err) => break Err(err.into()),
        }
    };
    fs.forget(parent, 1);
    let entry = found?;
    let name = entry.name().as_bytes();
    Ok(match wording {
        Wording::Fat => std::str::from_utf8(name)
            .context("Directory entry name is not valid UTF-8")?
            .to_string(),
        Wording::Optical => String::from_utf8_lossy(name).into_owned(),
    })
}

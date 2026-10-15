//! Flags and helpers every format's commands share.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use hadris_fs::host::{self, OnError, TreeOptions};
use hadris_fs::{Clock, DateTime, FileType, NodeId, Report, SystemClock, Tree, WarningKind};
use hadris_storage::Partition;
use hadris_storage::host::FileDevice;
use hadris_storage::sync::BlockDevice;
use hadris_udf::UdfRevision;

use crate::output::Output;

/// Where a `create` command writes its image or archive.
#[derive(Debug, Clone, clap::Args)]
pub struct Target {
    /// Output path; an existing file or device is refused without --force
    #[arg(short, long)]
    pub output: PathBuf,
    /// Replace an existing output file, or write over an existing device
    #[arg(short, long)]
    pub force: bool,
}

impl Target {
    /// Starts writing the output under the shared overwrite rule.
    pub fn create(&self) -> Result<(std::fs::File, Output)> {
        Output::create(&self.output, self.force)
            .with_context(|| format!("cannot create {}", self.output.display()))
    }

    /// Writes an image to the output with `write`, padding a regular file to
    /// at least `min_len` bytes. The file appears only once it is complete.
    pub fn write_image<T>(
        &self,
        min_len: u64,
        write: impl FnOnce(&mut FileDevice) -> Result<T>,
    ) -> Result<T> {
        let output = &self.output;
        let (file, pending) = self.create()?;
        let mut dev =
            FileDevice::new(file).with_context(|| format!("cannot create {}", output.display()))?;
        let written = write(&mut dev)?;
        let file = dev.into_inner();
        if pending.is_regular() && file.metadata()?.len() < min_len {
            file.set_len(min_len)?;
        }
        pending
            .commit(file)
            .with_context(|| format!("cannot write {}", output.display()))?;
        Ok(written)
    }
}

/// The window of an image file a read command mounts.
pub type Device = Partition<FileDevice>;

/// Where in an image or disk the volume a command reads starts.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct Locate {
    /// Partition table slot of the volume: the GPT entry index, the MBR slot
    /// (0 to 3), or 4 and up for MBR logical partitions
    #[arg(long, conflicts_with = "offset")]
    pub partition: Option<usize>,
    /// Byte offset of the volume in the image, a multiple of the block size
    #[arg(long)]
    pub offset: Option<u64>,
}

impl Locate {
    /// Opens `image` read-only and selects the volume's window of it.
    pub fn open(&self, image: &Path) -> Result<Device> {
        let dev =
            FileDevice::open(image).with_context(|| format!("cannot open {}", image.display()))?;
        self.select(dev)
            .with_context(|| format!("cannot locate the volume in {}", image.display()))
    }

    /// The window of `dev` the flags select: a partition, the bytes from an
    /// offset, or the whole device.
    pub fn select(&self, mut dev: FileDevice) -> Result<Device> {
        if let Some(slot) = self.partition {
            let disk = hadris::part::sync::read(&mut dev).context("cannot read partition table")?;
            let entry = disk
                .partition(slot)
                .with_context(|| format!("no partition in slot {slot}"))?;
            return Ok(hadris::part::sync::open(dev, &entry)?);
        }
        let block = u64::from(dev.block_size().get());
        let len = dev.block_count() * block;
        let offset = self.offset.unwrap_or(0);
        if offset % block != 0 {
            bail!("offset {offset} is not a multiple of the {block}-byte block size");
        }
        if offset > len {
            bail!("offset {offset} is past the end of the {len}-byte image");
        }
        Ok(Partition::new(dev, offset, len - offset))
    }
}

/// Reads the host directory `source` into a tree. Entries that cannot be
/// read are skipped with a warning.
pub fn read_source(source: &Path) -> Result<Tree> {
    let meta = std::fs::metadata(source)
        .with_context(|| format!("cannot read source {}", source.display()))?;
    if !meta.is_dir() {
        bail!("source is not a directory: {}", source.display());
    }
    let (tree, skipped) = host::read_tree(source, &TreeOptions::new().with_on_error(OnError::Skip))
        .with_context(|| format!("cannot read source {}", source.display()))?;
    for err in skipped {
        eprintln!("warning: skipped {err}");
    }
    Ok(tree)
}

/// The time new images are dated with: `SOURCE_DATE_EPOCH` when set, the
/// system time otherwise.
pub fn build_time() -> Result<DateTime> {
    Ok(host::source_date_epoch()?.unwrap_or_else(|| SystemClock.now()))
}

/// The deepest directory level a recursive walk enters, the limit of
/// `hadris_fs::read_tree` and `hadris_fs::Walk`.
pub const MAX_WALK_DEPTH: usize = 1024;

/// The directories from the top of a recursive walk down to the one being
/// listed, so that a damaged image whose directory leads back to one of them
/// fails instead of looping, and a walk stops at [`MAX_WALK_DEPTH`] levels.
#[derive(Debug)]
pub struct Ancestors(Vec<NodeId>);

impl Ancestors {
    /// The ancestors of a walk starting from the directory `top`.
    pub fn new(top: NodeId) -> Self {
        Self(vec![top])
    }

    /// Descends into the directory `node` at `path`, failing when it is
    /// already on the walk's path or the walk is too deep. A successful
    /// call is paired with [`Ancestors::leave`].
    pub fn enter(&mut self, node: NodeId, path: &str) -> Result<()> {
        if self.0.contains(&node) {
            bail!("Directory loop at {path}: corrupt filesystem data");
        }
        if self.0.len() >= MAX_WALK_DEPTH {
            bail!("Directory nesting exceeds {MAX_WALK_DEPTH} levels at {path}: limit exceeded");
        }
        self.0.push(node);
        Ok(())
    }

    /// Returns from the directory the last [`Ancestors::enter`] entered.
    pub fn leave(&mut self) {
        self.0.pop();
    }
}

/// Prints a writer's warnings. Dropped metadata, which a format without room
/// for it reports for every entry, only when `verbose`.
pub fn print_warnings(report: &Report, verbose: bool) {
    for warning in report.warnings() {
        if verbose || !matches!(warning.kind(), WarningKind::Dropped(_)) {
            eprintln!("warning: {warning}");
        }
    }
}

/// `/`-joined image path of `name` in the directory `dir`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The `ls -l` type character of `file_type`.
pub fn type_char(file_type: FileType) -> char {
    match file_type {
        FileType::File => '-',
        FileType::Dir => 'd',
        FileType::Symlink => 'l',
        FileType::CharDevice => 'c',
        FileType::BlockDevice => 'b',
        FileType::Fifo => 'p',
        FileType::Socket => 's',
        _ => '?',
    }
}

/// Format a size in bytes to a human-readable string.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// A writable UDF revision: `1.02`, `1.50`, `2.00` or `2.01`.
#[derive(Debug, Clone, Copy)]
pub struct RevisionArg(pub UdfRevision);

impl FromStr for RevisionArg {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let revision = match value {
            "1.02" => UdfRevision::V1_02,
            "1.50" => UdfRevision::V1_50,
            "2.00" => UdfRevision::V2_00,
            "2.01" => UdfRevision::V2_01,
            _ => return Err("expected a writable revision: 1.02, 1.50, 2.00 or 2.01"),
        };
        Ok(Self(revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_parse_only_writable_values() {
        for (input, expected) in [
            ("1.02", UdfRevision::V1_02),
            ("1.50", UdfRevision::V1_50),
            ("2.00", UdfRevision::V2_00),
            ("2.01", UdfRevision::V2_01),
        ] {
            assert_eq!(input.parse::<RevisionArg>().unwrap().0, expected);
        }
        for input in ["2.50", "2.60", "9.99", "1.03", "2.5", "abc", ""] {
            assert!(input.parse::<RevisionArg>().is_err(), "{input:?}");
        }
    }
}

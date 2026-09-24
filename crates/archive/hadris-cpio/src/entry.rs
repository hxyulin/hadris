use alloc::string::String;
use alloc::vec::Vec;

use hadris_fs::tree::{Content, Warning};
use hadris_fs::{DeviceKind, DeviceNumber};

/// What `CpioWriter::append` writes under a name.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum NewEntry<'a> {
    /// A regular file with its contents.
    File(&'a Content),
    /// A directory. Its children are separate entries.
    Dir,
    /// A symbolic link to a non-empty target.
    Symlink(&'a [u8]),
    /// A device node.
    Device(DeviceKind, DeviceNumber),
    /// A named pipe.
    Fifo,
    /// A Unix domain socket.
    Socket,
}

/// What a cpio writer wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    entries: u64,
    size_bytes: u64,
    warnings: Vec<Warning>,
}

impl Report {
    pub(crate) fn new(entries: u64, size_bytes: u64, warnings: Vec<Warning>) -> Self {
        Self {
            entries,
            size_bytes,
            warnings,
        }
    }

    /// Entries written, not counting the trailer.
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Bytes written, with the trailer when the archive was finished.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Metadata the format could not store: times other than the
    /// modification time, sub-second parts, attributes.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }
}

/// Lists what of `meta` a cpio header drops, for a warning.
pub(crate) fn dropped(meta: &hadris_fs::SetMetadata) -> Option<String> {
    let times = meta.times();
    let mut parts: Vec<&str> = Vec::new();
    if times.created().is_some() {
        parts.push("creation time");
    }
    if times.accessed().is_some() {
        parts.push("access time");
    }
    if times.changed().is_some() {
        parts.push("change time");
    }
    if times.modified().is_some_and(|time| time.nanoseconds() != 0) {
        parts.push("sub-second modification time");
    }
    if meta
        .attributes()
        .is_some_and(|attributes| !attributes.is_empty())
    {
        parts.push("attributes");
    }
    if parts.is_empty() {
        return None;
    }
    Some(alloc::format!(
        "cpio does not store the {}",
        parts.join(", ")
    ))
}

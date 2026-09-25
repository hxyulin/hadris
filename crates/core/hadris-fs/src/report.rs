//! What every writer, planner and `copy_tree` reports.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::fmt;

use crate::{Extent, Field, Name};

/// What kind of loss a [`Warning`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarningKind {
    /// The name was mapped to the format's rules (ISO 9660 d-characters,
    /// Joliet's forbidden characters, FAT characters);
    /// [`Warning::stored_as`] gives the result.
    Renamed,
    /// The name was cut to the format's length limit.
    Truncated,
    /// A name collided with another after mapping and got a unique suffix.
    Deduplicated,
    /// A directory was moved to the relocation directory, as Rock Ridge
    /// does with directories nested too deep.
    Relocated,
    /// The format does not store this field, or not all of it.
    Dropped(Field),
    /// The node was not written: the target cannot hold it.
    Skipped,
    /// A boot setup that will likely not boot on some firmware.
    Boot,
}

/// One thing a writer did differently than the tree asked, without failing.
///
/// Losses inherent to a format (FAT stores no owner) come once per kind,
/// with no path and the number of nodes affected; the rest come once per
/// node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Warning {
    kind: WarningKind,
    message: &'static str,
    path: Option<Vec<u8>>,
    stored_as: Option<Vec<u8>>,
    count: u64,
}

impl Warning {
    /// A warning of `kind` about one node. `message` says which rule or
    /// namespace, such as `"joliet name"`.
    pub const fn new(kind: WarningKind, message: &'static str) -> Self {
        Self {
            kind,
            message,
            path: None,
            stored_as: None,
            count: 1,
        }
    }

    /// Records the tree path of the node.
    #[must_use]
    pub fn with_path(mut self, path: impl AsRef<[u8]>) -> Self {
        self.path = Some(path.as_ref().to_vec());
        self
    }

    /// Records the name as the format stored it.
    #[must_use]
    pub fn with_stored_as(mut self, name: impl AsRef<[u8]>) -> Self {
        self.stored_as = Some(name.as_ref().to_vec());
        self
    }

    /// Records how many nodes a per-kind warning covers.
    #[must_use]
    pub const fn with_count(mut self, count: u64) -> Self {
        self.count = count;
        self
    }

    /// What happened.
    pub fn kind(&self) -> WarningKind {
        self.kind
    }

    /// Which rule or namespace, such as `"joliet name"` or
    /// `"iso level 1 name"`.
    pub fn message(&self) -> &'static str {
        self.message
    }

    /// The tree path, or `None` for a warning about a whole kind of loss.
    pub fn path(&self) -> Option<&[u8]> {
        self.path.as_deref()
    }

    /// The name as written, for renames, truncations, deduplications and
    /// relocations.
    pub fn stored_as(&self) -> Option<&[u8]> {
        self.stored_as.as_deref()
    }

    /// The nodes affected: 1 for a warning about one node.
    pub fn count(&self) -> u64 {
        self.count
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            WarningKind::Dropped(field) => write!(f, "{field:?} dropped")?,
            kind => write!(f, "{kind:?}")?,
        }
        write!(f, " ({})", self.message)?;
        if let Some(path) = &self.path {
            write!(f, ": {}", Name::new(path))?;
        } else if self.count != 1 {
            write!(f, ": {} nodes", self.count)?;
        }
        if let Some(name) = &self.stored_as {
            write!(f, " as {}", Name::new(name))?;
        }
        Ok(())
    }
}

/// A tree path as reports key it: its components joined by `/`, with no
/// leading, trailing or repeated `/` and no leading `./`.
fn normalize(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len());
    for (index, part) in path.split(|&byte| byte == b'/').enumerate() {
        if part.is_empty() || (index == 0 && part == b".") {
            continue;
        }
        if !out.is_empty() {
            out.push(b'/');
        }
        out.extend_from_slice(part);
    }
    out
}

/// What a writer, a planner or `copy_tree` returns: the size of the output,
/// every loss, and where each file's data went.
///
/// A plan returns exactly the report the write returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    size: u64,
    warnings: Vec<Warning>,
    extents: BTreeMap<Vec<u8>, Vec<Extent>>,
}

impl Report {
    /// An empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes of output: the image, or the archive segment. 0 for
    /// `copy_tree`.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// What the writer did differently than the tree asked.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The extents of a file's data, in output order: one for most files,
    /// several for an ISO 9660 file of 4 GiB or more. `None` for unknown
    /// paths and files without data. Hard link names share extents.
    pub fn extents(&self, path: impl AsRef<[u8]>) -> Option<&[Extent]> {
        self.extents
            .get(&normalize(path.as_ref()))
            .map(Vec::as_slice)
    }

    /// Every file with data and its extents, by path, sorted by path bytes.
    pub fn files(&self) -> impl Iterator<Item = (&Name, &[Extent])> + '_ {
        self.extents
            .iter()
            .map(|(path, extents)| (Name::new(path), extents.as_slice()))
    }

    /// Sets the size of the output.
    pub fn set_size(&mut self, size: u64) {
        self.size = size;
    }

    /// Adds a warning.
    pub fn push_warning(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Adds `extent` to the extents of the file at `path`.
    pub fn push_extent(&mut self, path: impl AsRef<[u8]>, extent: Extent) {
        self.extents
            .entry(normalize(path.as_ref()))
            .or_default()
            .push(extent);
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes, {} files", self.size, self.extents.len())?;
        for warning in &self.warnings {
            write!(f, "\nwarning: {warning}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extents_are_found_by_any_spelling_of_the_path() {
        let mut report = Report::new();
        report.set_size(4096);
        report.push_extent("/boot//grub.cfg", Extent::new(2048, 10));
        report.push_extent("boot/grub.cfg", Extent::new(8192, 3));
        assert_eq!(report.extents("./boot/grub.cfg").unwrap().len(), 2);
        assert_eq!(
            report.extents("boot/grub.cfg/"),
            report.extents("/boot/grub.cfg")
        );
        assert!(report.extents("boot").is_none());
        let files: Vec<_> = report
            .files()
            .map(|(path, _)| path.as_bytes().to_vec())
            .collect();
        assert_eq!(files, [b"boot/grub.cfg".to_vec()]);
    }

    #[test]
    fn warnings_display_their_path_or_count() {
        let mut report = Report::new();
        report.push_warning(
            Warning::new(WarningKind::Renamed, "iso level 1 name")
                .with_path("/Read Me.txt")
                .with_stored_as("READ_ME.TXT;1"),
        );
        report.push_warning(Warning::new(WarningKind::Dropped(Field::Owner), "fat").with_count(3));
        let text = alloc::format!("{report}");
        assert_eq!(
            text,
            "0 bytes, 0 files\n\
             warning: Renamed (iso level 1 name): /Read Me.txt as READ_ME.TXT;1\n\
             warning: Owner dropped (fat): 3 nodes"
        );
    }
}

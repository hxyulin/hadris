use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use hadris_fs::Extent;
use hadris_fs::tree::Warning;

/// What `write` produced, or `plan` would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    total_blocks: u64,
    extents: BTreeMap<String, Extent>,
    warnings: Vec<Warning>,
}

/// A tree path in the form reports key it by: `/` separated, with a
/// leading `/` and no empty components.
pub(crate) fn normalize(path: &str) -> String {
    let mut out = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        out.push('/');
        out.push_str(part);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

impl Report {
    pub(crate) fn new(
        total_blocks: u64,
        extents: BTreeMap<String, Extent>,
        warnings: Vec<Warning>,
    ) -> Self {
        Self {
            total_blocks,
            extents,
            warnings,
        }
    }

    pub(crate) fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// The image length in 2048-byte blocks, including a backup GPT and,
    /// for a session, the sessions before it.
    pub fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    /// The image length in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.total_blocks * crate::raw::SECTOR_SIZE as u64
    }

    /// What the writer did differently than the tree asked: metadata the
    /// image cannot store, entries it left out, names it had to change.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Where the file at tree path `path` is stored: its first byte and
    /// its length. The extents of a file larger than 4 GiB are contiguous,
    /// so one range covers them. Empty files and paths that are not files
    /// give `None`.
    pub fn extent_of(&self, path: &str) -> Option<Extent> {
        self.extents.get(&normalize(path)).copied()
    }

    /// Every stored file and its extent, by normalized tree path.
    pub fn extents(&self) -> impl Iterator<Item = (&str, Extent)> + '_ {
        self.extents
            .iter()
            .map(|(path, extent)| (path.as_str(), *extent))
    }
}

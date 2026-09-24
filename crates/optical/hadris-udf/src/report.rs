use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use hadris_fs::Extent;
use hadris_fs::tree::Warning;

/// What `write` produced, or `plan` would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    total_blocks: u64,
    allocated_end: u64,
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
        allocated_end: u64,
        extents: BTreeMap<String, Extent>,
        warnings: Vec<Warning>,
    ) -> Self {
        Self {
            total_blocks,
            allocated_end,
            extents,
            warnings,
        }
    }

    /// The volume length in 2048-byte blocks, the trailing anchor and the
    /// 256 blocks after it included.
    pub fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    /// The volume length in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.total_blocks * 2048
    }

    /// The block after the last one the writer allocates in the partition
    /// for its structures and the data it writes. In a bridge volume the
    /// ISO 9660 directories and files go at or after it.
    pub fn allocated_end(&self) -> u64 {
        self.allocated_end
    }

    /// What the writer did differently than the tree asked: metadata the
    /// volume cannot store and entries it left out.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Where the file at tree path `path` is stored: its first byte and its
    /// length. Empty files, files whose extents are not contiguous and paths
    /// that are not files give `None`.
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

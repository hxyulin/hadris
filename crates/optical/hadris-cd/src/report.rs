use alloc::vec::Vec;

use hadris_fs::Extent;
use hadris_fs::tree::Warning;

/// What `write` produced, or `plan` would produce: the reports of both
/// writers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    iso: hadris_iso::Report,
    udf: hadris_udf::Report,
    warnings: Vec<Warning>,
}

impl Report {
    pub(crate) fn new(iso: hadris_iso::Report, udf: hadris_udf::Report) -> Self {
        let warnings = iso
            .warnings()
            .iter()
            .chain(udf.warnings())
            .cloned()
            .collect();
        Self { iso, udf, warnings }
    }

    /// The image length in 2048-byte blocks.
    pub fn total_blocks(&self) -> u64 {
        self.udf.total_blocks().max(self.iso.total_blocks())
    }

    /// The image length in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.total_blocks() * 2048
    }

    /// The warnings of the ISO 9660 writer, then those of the UDF writer.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Where the file at tree path `path` is stored, for both trees.
    pub fn extent_of(&self, path: &str) -> Option<Extent> {
        self.iso.extent_of(path)
    }

    /// The report of the ISO 9660 writer.
    pub fn iso(&self) -> &hadris_iso::Report {
        &self.iso
    }

    /// The report of the UDF writer.
    pub fn udf(&self) -> &hadris_udf::Report {
        &self.udf
    }
}

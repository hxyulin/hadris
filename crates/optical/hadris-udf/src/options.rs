use alloc::string::String;

use hadris_fs::{DateTime, NoClock};

use crate::UdfRevision;
use crate::volume::UdfId;

/// Options for writing a UDF volume.
///
/// The defaults write a UDF 1.02 volume named `UDF_VOLUME`, dated
/// 1980-01-01 ([`NoClock::TIME`]), so the same tree always gives the same
/// bytes.
///
/// ```rust
/// use hadris_udf::{UdfId, UdfOptions, UdfRevision};
///
/// let options = UdfOptions::new()
///     .with_id(UdfId::Volume, "MOVIES")
///     .with_revision(UdfRevision::V2_01);
/// assert_eq!(options.id(UdfId::Volume), Some("MOVIES"));
/// ```
#[derive(Debug, Clone)]
pub struct UdfOptions {
    ids: [Option<String>; 4],
    revision: UdfRevision,
    min_blocks: u64,
    time: DateTime,
    seed: Option<u64>,
}

impl Default for UdfOptions {
    fn default() -> Self {
        let mut ids = [const { None }; 4];
        ids[UdfId::Volume.index()] = Some(String::from("UDF_VOLUME"));
        Self {
            ids,
            revision: UdfRevision::V1_02,
            min_blocks: 0,
            time: NoClock::TIME,
            seed: None,
        }
    }
}

impl UdfOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an identifier. One longer than its field fails the plan with
    /// [`Detail::Identifier`](crate::Detail::Identifier); characters
    /// outside Latin-1 take two bytes each.
    pub fn with_id(mut self, id: UdfId, value: &str) -> Self {
        self.ids[id.index()] = Some(String::from(value));
        self
    }

    /// Sets the UDF revision the volume records: 1.02 to 2.01. 2.00 and
    /// later write ECMA-167 3rd edition structures (NSR03, descriptor
    /// version 3). 2.50 and later require a metadata partition (UDF 2.50
    /// 2.2.10), which the writer does not write, so `plan` and `write` fail
    /// with [`ErrorKind::Unsupported`](hadris_fs::ErrorKind::Unsupported)
    /// and [`Detail::PartitionMap`](crate::Detail::PartitionMap).
    pub fn with_revision(self, revision: UdfRevision) -> Self {
        Self { revision, ..self }
    }

    /// Makes the volume at least `blocks` 2048-byte blocks long; the
    /// partition then runs to the anchor 257 blocks before the end, leaving
    /// free space after the files.
    pub fn with_min_blocks(self, blocks: u64) -> Self {
        Self {
            min_blocks: blocks,
            ..self
        }
    }

    /// Sets the time that dates the volume and the entries without times,
    /// such as `SOURCE_DATE_EPOCH`. The writer reads no clock.
    pub fn with_time(self, time: DateTime) -> Self {
        Self { time, ..self }
    }

    /// Sets the seed the volume serial derives from, with the tree's
    /// paths, sizes and times. Without one it derives from the time and the
    /// tree, so the same inputs give the same volume and different trees
    /// different serials. [`UdfId::VolumeSet`] overrides it.
    pub fn with_seed(self, seed: u64) -> Self {
        Self {
            seed: Some(seed),
            ..self
        }
    }

    /// An identifier as set, or `None` when it takes its default.
    pub fn id(&self, id: UdfId) -> Option<&str> {
        self.ids[id.index()].as_deref()
    }

    /// The UDF revision.
    pub fn revision(&self) -> UdfRevision {
        self.revision
    }

    /// The smallest volume length in blocks.
    pub fn min_blocks(&self) -> u64 {
        self.min_blocks
    }

    /// The time that dates the volume and the entries without times.
    pub fn time(&self) -> DateTime {
        self.time
    }

    /// The seed of the volume serial, if set.
    pub fn seed(&self) -> Option<u64> {
        self.seed
    }
}

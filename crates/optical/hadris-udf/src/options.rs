use alloc::string::String;

use hadris_fs::{DateTime, NoClock};

use crate::UdfRevision;

/// Options for writing a UDF volume.
///
/// The defaults write a UDF 1.02 volume named `UDF_VOLUME`, dated
/// 1980-01-01 ([`NoClock::TIME`]), so the same tree always gives the same
/// bytes.
///
/// ```rust
/// use hadris_udf::{UdfOptions, UdfRevision};
///
/// let options = UdfOptions::default()
///     .with_volume_id("MOVIES")
///     .with_revision(UdfRevision::V2_01);
/// assert_eq!(options.volume_id(), "MOVIES");
/// ```
#[derive(Debug, Clone)]
pub struct UdfOptions {
    volume_id: String,
    revision: UdfRevision,
    min_blocks: u64,
    time: DateTime,
}

impl Default for UdfOptions {
    fn default() -> Self {
        Self {
            volume_id: String::from("UDF_VOLUME"),
            revision: UdfRevision::V1_02,
            min_blocks: 0,
            time: NoClock::TIME,
        }
    }
}

impl UdfOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the volume identifier. It names the logical volume and the file
    /// set, up to 126 bytes of OSTA Compressed Unicode, and is cut to 30
    /// bytes for the primary volume descriptor and file set identifier.
    pub fn with_volume_id(self, volume_id: impl Into<String>) -> Self {
        Self {
            volume_id: volume_id.into(),
            ..self
        }
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
    /// partition then runs to the anchor 257 blocks before the end.
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

    /// The volume identifier.
    pub fn volume_id(&self) -> &str {
        &self.volume_id
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
}

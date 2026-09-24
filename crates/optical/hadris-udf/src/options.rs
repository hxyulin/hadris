use alloc::string::String;

use hadris_fs::{Clock, NoClock};

use crate::UdfRevision;

/// Makes a UDF volume share its image with ISO 9660, as the UDF Bridge
/// format of DVD-Video and `hadris-cd` does.
///
/// The ISO 9660 image is written first. The UDF volume then records its
/// recognition sequence after the ISO volume descriptors, keeps its
/// structures before the blocks
/// [`Report::allocated_end`](crate::Report::allocated_end) of a
/// [`plan`](crate::sync::plan) gives, and points each file at data already
/// on the device: file contents must be
/// [`Content::stored`](hadris_fs::tree::Content::stored) extents there, or
/// empty. Nothing else of the image is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bridge {
    iso_descriptors: u32,
}

impl Bridge {
    /// A bridge after `iso_descriptors` ISO 9660 volume descriptors at
    /// logical sector 16, the set terminator included.
    pub const fn new(iso_descriptors: u32) -> Self {
        Self { iso_descriptors }
    }

    /// The number of ISO 9660 volume descriptors before the UDF
    /// recognition sequence.
    pub const fn iso_descriptors(&self) -> u32 {
        self.iso_descriptors
    }
}

/// Options for writing a UDF volume.
///
/// The defaults write a UDF 1.02 volume named `UDF_VOLUME`, dated by
/// [`NoClock`], so the same tree always gives the same bytes.
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
pub struct UdfOptions<C = NoClock> {
    volume_id: String,
    revision: UdfRevision,
    min_blocks: u64,
    bridge: Option<Bridge>,
    clock: C,
}

impl Default for UdfOptions {
    fn default() -> Self {
        Self {
            volume_id: String::from("UDF_VOLUME"),
            revision: UdfRevision::V1_02,
            min_blocks: 0,
            bridge: None,
            clock: NoClock,
        }
    }
}

impl UdfOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: Clock> UdfOptions<C> {
    /// Sets the volume identifier. It names the logical volume and the file
    /// set, up to 126 bytes of OSTA Compressed Unicode, and is cut to 30
    /// bytes for the primary volume descriptor and file set identifier.
    pub fn with_volume_id(self, volume_id: impl Into<String>) -> Self {
        Self {
            volume_id: volume_id.into(),
            ..self
        }
    }

    /// Sets the UDF revision the volume records. 2.00 and later write
    /// ECMA-167 3rd edition structures (NSR03, descriptor version 3).
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

    /// Writes a bridge volume over an ISO 9660 image on the same device.
    pub fn with_bridge(self, bridge: Bridge) -> Self {
        Self {
            bridge: Some(bridge),
            ..self
        }
    }

    /// Sets the clock that dates the volume and the entries without times.
    pub fn with_clock<C2: Clock>(self, clock: C2) -> UdfOptions<C2> {
        UdfOptions {
            volume_id: self.volume_id,
            revision: self.revision,
            min_blocks: self.min_blocks,
            bridge: self.bridge,
            clock,
        }
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

    /// The bridge, for a volume that shares an ISO 9660 image.
    pub fn bridge(&self) -> Option<Bridge> {
        self.bridge
    }

    /// The clock.
    pub fn clock(&self) -> &C {
        &self.clock
    }
}

//! What exFAT `check` reports: [`Finding`]s one at a time and a
//! [`CheckReport`] of counts at the end.

/// One problem `check` found on an exFAT volume.
///
/// Locations are byte offsets on the volume. `entry` is the offset of the
/// first entry of an entry set, or of a single entry; the root directory,
/// which has no entry, is named by entry 0.
///
/// Findings are made only by `check`. Variants with fields are
/// `#[non_exhaustive]`, so a later release can locate a problem more
/// precisely; match them with `..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Finding {
    /// A boot sector field is inconsistent with the rest of the volume; the
    /// text names it. The volume mounted, so the fields `ExFatFs` relies on
    /// are sound.
    BootSector(&'static str),
    /// The checksum sector of the main boot region does not match the
    /// region.
    BootChecksum,
    /// The backup boot region differs from the main one, `VolumeFlags` and
    /// `PercentInUse` aside.
    BackupBootRegion,
    /// `VolumeFlags` marks the volume dirty.
    VolumeDirty,
    /// `PercentInUse` is neither unknown nor the share of allocated
    /// clusters.
    #[non_exhaustive]
    PercentInUse {
        /// The value in the boot sector.
        recorded: u8,
        /// The share of clusters the bitmap marks allocated.
        actual: u8,
    },
    /// FAT entries 0 and 1 do not hold the media type and `0xFFFFFFFF`.
    FatEntries,
    /// The up-case table does not match its checksum, or maps one of the
    /// first 128 code points other than the specification requires.
    UpcaseTable,
    /// An Allocation Bitmap, Up-case Table or Volume Label entry outside
    /// the root directory, a second one in it, or a label longer than 11
    /// characters.
    #[non_exhaustive]
    RootEntry {
        /// The entry.
        entry: u64,
    },
    /// An entry set whose `SetChecksum` does not match its entries.
    #[non_exhaustive]
    SetChecksum {
        /// The set's File entry.
        entry: u64,
    },
    /// A File entry that is not followed by a Stream Extension entry and
    /// enough File Name entries, or secondary entries that follow no
    /// primary entry.
    #[non_exhaustive]
    EntrySet {
        /// The first entry.
        entry: u64,
    },
    /// A name that is empty, holds a character exFAT forbids, or is `.` or
    /// `..`.
    #[non_exhaustive]
    BadName {
        /// The set's File entry.
        entry: u64,
    },
    /// A `NameHash` that is not the hash of the up-cased name.
    #[non_exhaustive]
    NameHash {
        /// The set's File entry.
        entry: u64,
    },
    /// A directory whose `DataLength` is not a whole number of clusters,
    /// above 256 MiB, or different from its `ValidDataLength`.
    #[non_exhaustive]
    DirectorySize {
        /// The set's File entry.
        entry: u64,
    },
    /// A file whose `ValidDataLength` exceeds its `DataLength`.
    #[non_exhaustive]
    ValidDataLength {
        /// The set's File entry.
        entry: u64,
    },
    /// The entry's first cluster is not a heap cluster, or is missing for
    /// a non-empty allocation.
    #[non_exhaustive]
    InvalidCluster {
        /// The entry.
        entry: u64,
        /// The cluster it names.
        cluster: u32,
    },
    /// The chain links `cluster` to a free, reserved or out-of-range value.
    #[non_exhaustive]
    BrokenChain {
        /// The entry that owns the chain.
        entry: u64,
        /// The last cluster that links correctly.
        cluster: u32,
        /// Its FAT entry.
        next: u32,
    },
    /// The chain runs into a cluster marked bad.
    #[non_exhaustive]
    BadCluster {
        /// The entry that owns the chain.
        entry: u64,
        /// The bad cluster.
        cluster: u32,
    },
    /// The chain links back to one of its own clusters.
    #[non_exhaustive]
    CyclicChain {
        /// The entry that owns the chain.
        entry: u64,
        /// The cluster whose link closes the cycle.
        cluster: u32,
    },
    /// The allocation has more clusters than its `DataLength` needs.
    #[non_exhaustive]
    ChainTooLong {
        /// The entry.
        entry: u64,
        /// The `DataLength`.
        size: u64,
        /// The clusters in the chain.
        clusters: u32,
    },
    /// The allocation has fewer clusters than its `DataLength` needs.
    #[non_exhaustive]
    ChainTooShort {
        /// The entry.
        entry: u64,
        /// The `DataLength`.
        size: u64,
        /// The clusters in the chain.
        clusters: u32,
    },
    /// A cluster is in the allocations of more than one entry. Reported
    /// once for each claim after the first.
    #[non_exhaustive]
    CrossLinked {
        /// The entry whose allocation claimed the cluster again.
        entry: u64,
        /// The cluster.
        cluster: u32,
    },
    /// Clusters the bitmap marks allocated that no allocation reaches,
    /// `first..first + count`.
    #[non_exhaustive]
    LostClusters {
        /// The first lost cluster of the run.
        first: u32,
        /// The number of consecutive lost clusters.
        count: u32,
    },
    /// Clusters an allocation uses that the bitmap marks free,
    /// `first..first + count`.
    #[non_exhaustive]
    FreeInUse {
        /// The first cluster of the run.
        first: u32,
        /// The number of consecutive clusters.
        count: u32,
    },
    /// A directory more than 64 levels below the root, which `check` does
    /// not enter; what it holds is reported as lost.
    #[non_exhaustive]
    TooDeep {
        /// The directory's File entry.
        entry: u64,
    },
}

/// The kind of a [`Finding`], for counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum FindingKind {
    /// [`Finding::BootSector`].
    BootSector,
    /// [`Finding::BootChecksum`].
    BootChecksum,
    /// [`Finding::BackupBootRegion`].
    BackupBootRegion,
    /// [`Finding::VolumeDirty`].
    VolumeDirty,
    /// [`Finding::PercentInUse`].
    PercentInUse,
    /// [`Finding::FatEntries`].
    FatEntries,
    /// [`Finding::UpcaseTable`].
    UpcaseTable,
    /// [`Finding::RootEntry`].
    RootEntry,
    /// [`Finding::SetChecksum`].
    SetChecksum,
    /// [`Finding::EntrySet`].
    EntrySet,
    /// [`Finding::BadName`].
    BadName,
    /// [`Finding::NameHash`].
    NameHash,
    /// [`Finding::DirectorySize`].
    DirectorySize,
    /// [`Finding::ValidDataLength`].
    ValidDataLength,
    /// [`Finding::InvalidCluster`].
    InvalidCluster,
    /// [`Finding::BrokenChain`].
    BrokenChain,
    /// [`Finding::BadCluster`].
    BadCluster,
    /// [`Finding::CyclicChain`].
    CyclicChain,
    /// [`Finding::ChainTooLong`].
    ChainTooLong,
    /// [`Finding::ChainTooShort`].
    ChainTooShort,
    /// [`Finding::CrossLinked`].
    CrossLinked,
    /// [`Finding::LostClusters`].
    LostClusters,
    /// [`Finding::FreeInUse`].
    FreeInUse,
    /// [`Finding::TooDeep`].
    TooDeep,
}

const KINDS: usize = FindingKind::TooDeep as usize + 1;

impl Finding {
    /// The kind of this finding.
    pub const fn kind(&self) -> FindingKind {
        match self {
            Self::BootSector(_) => FindingKind::BootSector,
            Self::BootChecksum => FindingKind::BootChecksum,
            Self::BackupBootRegion => FindingKind::BackupBootRegion,
            Self::VolumeDirty => FindingKind::VolumeDirty,
            Self::PercentInUse { .. } => FindingKind::PercentInUse,
            Self::FatEntries => FindingKind::FatEntries,
            Self::UpcaseTable => FindingKind::UpcaseTable,
            Self::RootEntry { .. } => FindingKind::RootEntry,
            Self::SetChecksum { .. } => FindingKind::SetChecksum,
            Self::EntrySet { .. } => FindingKind::EntrySet,
            Self::BadName { .. } => FindingKind::BadName,
            Self::NameHash { .. } => FindingKind::NameHash,
            Self::DirectorySize { .. } => FindingKind::DirectorySize,
            Self::ValidDataLength { .. } => FindingKind::ValidDataLength,
            Self::InvalidCluster { .. } => FindingKind::InvalidCluster,
            Self::BrokenChain { .. } => FindingKind::BrokenChain,
            Self::BadCluster { .. } => FindingKind::BadCluster,
            Self::CyclicChain { .. } => FindingKind::CyclicChain,
            Self::ChainTooLong { .. } => FindingKind::ChainTooLong,
            Self::ChainTooShort { .. } => FindingKind::ChainTooShort,
            Self::CrossLinked { .. } => FindingKind::CrossLinked,
            Self::LostClusters { .. } => FindingKind::LostClusters,
            Self::FreeInUse { .. } => FindingKind::FreeInUse,
            Self::TooDeep { .. } => FindingKind::TooDeep,
        }
    }
}

/// The totals of an exFAT `check` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub(crate) counts: [u32; KINDS],
    pub(crate) files: u32,
    pub(crate) directories: u32,
    pub(crate) used: u32,
    pub(crate) free: u32,
    pub(crate) bad: u32,
    pub(crate) lost: u32,
    pub(crate) passes: u32,
}

#[cfg(any(feature = "sync", feature = "async"))]
impl CheckReport {
    pub(crate) const fn new() -> Self {
        Self {
            counts: [0; KINDS],
            files: 0,
            directories: 0,
            used: 0,
            free: 0,
            bad: 0,
            lost: 0,
            passes: 0,
        }
    }

    pub(crate) fn record(&mut self, finding: &Finding) {
        let count = &mut self.counts[finding.kind() as usize];
        *count = count.saturating_add(1);
    }
}

impl CheckReport {
    /// Whether nothing was found.
    pub fn is_clean(&self) -> bool {
        self.findings() == 0
    }

    /// The number of findings.
    pub fn findings(&self) -> u32 {
        self.counts.iter().fold(0, |sum, &n| sum.saturating_add(n))
    }

    /// The number of findings of `kind`.
    pub fn count(&self, kind: FindingKind) -> u32 {
        self.counts[kind as usize]
    }

    /// Files reached from the root.
    pub fn files(&self) -> u32 {
        self.files
    }

    /// Directories reached from the root, the root included.
    pub fn directories(&self) -> u32 {
        self.directories
    }

    /// Clusters the bitmap marks allocated, lost ones included.
    pub fn used_clusters(&self) -> u32 {
        self.used
    }

    /// Clusters the bitmap marks free.
    pub fn free_clusters(&self) -> u32 {
        self.free
    }

    /// Allocated clusters that no allocation reaches and the FAT marks bad.
    pub fn bad_clusters(&self) -> u32 {
        self.bad
    }

    /// Allocated clusters that no allocation reaches, bad ones excluded.
    pub fn lost_clusters(&self) -> u32 {
        self.lost
    }

    /// How many times the directory tree was walked: once for each
    /// bitmap's worth of clusters.
    pub fn passes(&self) -> u32 {
        self.passes
    }
}

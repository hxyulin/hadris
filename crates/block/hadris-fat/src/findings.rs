//! What `check` reports: [`Finding`]s one at a time and a [`CheckReport`]
//! of counts at the end.

/// One problem `check` found on a volume.
///
/// Locations are byte offsets on the volume. `entry` is the offset of a
/// directory entry: the short entry of a file or directory, or the first
/// slot of a long-name run. The root directory, which has no entry, is
/// named by entry 0, where only the boot sector can be.
///
/// Findings are made only by `check`. Variants with fields are
/// `#[non_exhaustive]`, so a later release can locate a problem more
/// precisely; match them with `..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Finding {
    /// A boot sector field is inconsistent with the rest of the volume; the
    /// text names it. The volume mounted, so the fields `FatFs` relies on
    /// are sound.
    BootSector(&'static str),
    /// The FAT32 backup boot sector differs from the boot sector.
    BackupBootSector,
    /// FAT entries 0 and 1 do not hold the media byte and an end-of-chain
    /// marker.
    ReservedEntries,
    /// The FAT32 FSInfo sector named by the boot sector has wrong
    /// signatures.
    FsInfo,
    /// The FSInfo free cluster count is neither unknown nor the number of
    /// free clusters in the FAT.
    #[non_exhaustive]
    FreeCount {
        /// The count in the FSInfo sector.
        recorded: u32,
        /// The free clusters in the FAT.
        actual: u32,
    },
    /// A mirrored FAT copy differs from the active copy.
    #[non_exhaustive]
    FatCopy {
        /// The index of the copy.
        copy: u8,
        /// The first cluster whose entry differs; 0 and 1 are the reserved
        /// entries.
        first: u32,
        /// How many entries differ.
        entries: u32,
    },
    /// The entry's first cluster is not a data cluster, is the FAT32 root,
    /// or is missing from a directory.
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
    /// A file's chain has more clusters than its size needs.
    #[non_exhaustive]
    ChainTooLong {
        /// The file's entry.
        entry: u64,
        /// The size in the entry.
        size: u32,
        /// The clusters in the chain.
        clusters: u32,
    },
    /// A file's chain has fewer clusters than its size needs.
    #[non_exhaustive]
    ChainTooShort {
        /// The file's entry.
        entry: u64,
        /// The size in the entry.
        size: u32,
        /// The clusters in the chain.
        clusters: u32,
    },
    /// A cluster is in the chains of more than one entry. Reported once for
    /// each claim after the first.
    #[non_exhaustive]
    CrossLinked {
        /// The entry whose chain claimed the cluster again.
        entry: u64,
        /// The cluster.
        cluster: u32,
    },
    /// Allocated clusters that no chain reaches, `first..first + count`.
    #[non_exhaustive]
    LostClusters {
        /// The first lost cluster of the run.
        first: u32,
        /// The number of consecutive lost clusters.
        count: u32,
    },
    /// A short name holds a byte FAT does not allow.
    #[non_exhaustive]
    BadName {
        /// The entry.
        entry: u64,
    },
    /// A `.` or `..` entry is missing, wrong or out of place. `entry` is the
    /// directory's own entry, or the stray dot entry. A directory whose
    /// `..` does not name its parent is not entered, so what it holds is
    /// reported as lost.
    #[non_exhaustive]
    DotEntry {
        /// The entry.
        entry: u64,
    },
    /// A directory entry records a non-zero size.
    #[non_exhaustive]
    DirectorySize {
        /// The entry.
        entry: u64,
    },
    /// A volume label entry outside the root directory, or a second one in
    /// it.
    #[non_exhaustive]
    Label {
        /// The entry.
        entry: u64,
    },
    /// A complete long-name run whose checksum does not match the short
    /// entry after it.
    #[non_exhaustive]
    LfnChecksum {
        /// The first slot of the run.
        entry: u64,
    },
    /// Long-name fragments that do not form a run ending in a short entry.
    #[non_exhaustive]
    OrphanLfn {
        /// The first fragment.
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
    /// [`Finding::BackupBootSector`].
    BackupBootSector,
    /// [`Finding::ReservedEntries`].
    ReservedEntries,
    /// [`Finding::FsInfo`].
    FsInfo,
    /// [`Finding::FreeCount`].
    FreeCount,
    /// [`Finding::FatCopy`].
    FatCopy,
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
    /// [`Finding::BadName`].
    BadName,
    /// [`Finding::DotEntry`].
    DotEntry,
    /// [`Finding::DirectorySize`].
    DirectorySize,
    /// [`Finding::Label`].
    Label,
    /// [`Finding::LfnChecksum`].
    LfnChecksum,
    /// [`Finding::OrphanLfn`].
    OrphanLfn,
}

const KINDS: usize = FindingKind::OrphanLfn as usize + 1;

impl Finding {
    /// The kind of this finding.
    pub const fn kind(&self) -> FindingKind {
        match self {
            Self::BootSector(_) => FindingKind::BootSector,
            Self::BackupBootSector => FindingKind::BackupBootSector,
            Self::ReservedEntries => FindingKind::ReservedEntries,
            Self::FsInfo => FindingKind::FsInfo,
            Self::FreeCount { .. } => FindingKind::FreeCount,
            Self::FatCopy { .. } => FindingKind::FatCopy,
            Self::InvalidCluster { .. } => FindingKind::InvalidCluster,
            Self::BrokenChain { .. } => FindingKind::BrokenChain,
            Self::BadCluster { .. } => FindingKind::BadCluster,
            Self::CyclicChain { .. } => FindingKind::CyclicChain,
            Self::ChainTooLong { .. } => FindingKind::ChainTooLong,
            Self::ChainTooShort { .. } => FindingKind::ChainTooShort,
            Self::CrossLinked { .. } => FindingKind::CrossLinked,
            Self::LostClusters { .. } => FindingKind::LostClusters,
            Self::BadName { .. } => FindingKind::BadName,
            Self::DotEntry { .. } => FindingKind::DotEntry,
            Self::DirectorySize { .. } => FindingKind::DirectorySize,
            Self::Label { .. } => FindingKind::Label,
            Self::LfnChecksum { .. } => FindingKind::LfnChecksum,
            Self::OrphanLfn { .. } => FindingKind::OrphanLfn,
        }
    }
}

/// The totals of a `check` run.
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

    /// Clusters the FAT marks as in use, lost ones included.
    pub fn used_clusters(&self) -> u32 {
        self.used
    }

    /// Clusters the FAT marks as free.
    pub fn free_clusters(&self) -> u32 {
        self.free
    }

    /// Clusters the FAT marks as bad.
    pub fn bad_clusters(&self) -> u32 {
        self.bad
    }

    /// Clusters in use that no chain reaches.
    pub fn lost_clusters(&self) -> u32 {
        self.lost
    }

    /// How many times the directory tree was walked: once for each
    /// bitmap's worth of clusters.
    pub fn passes(&self) -> u32 {
        self.passes
    }
}

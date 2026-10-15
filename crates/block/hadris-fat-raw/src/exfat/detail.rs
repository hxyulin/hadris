use core::fmt;

use hadris_fs::{DetailCode, Error, ErrorKind};

const DOMAIN: &str = "hadris-fat::exfat";

/// What exactly is wrong on an exFAT volume, shared by the errors of
/// mounting and reading and by the findings of `check`.
///
/// Read it back from an [`Error`] with [`Detail::of`], or from a finding
/// with [`Detail::from_code`]. Codes never change meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The boot sector is not a valid exFAT boot sector, or a field is
    /// inconsistent with the rest of the volume.
    BootSector = 1,
    /// The checksum sector of the main boot region does not match it.
    BootChecksum = 2,
    /// The backup boot region differs from the main one.
    BackupBootRegion = 3,
    /// The up-case table is missing, does not match its checksum, or maps
    /// one of the first 128 code points other than the specification
    /// requires.
    UpcaseTable = 4,
    /// An entry set whose `SetChecksum` does not match its entries.
    SetChecksum = 5,
    /// A `NameHash` that is not the hash of the up-cased name.
    NameHash = 6,
    /// A file whose `ValidDataLength` exceeds its `DataLength`.
    ValidDataLength = 7,
    /// The Allocation Bitmap is missing, too short, or marks clusters in
    /// use as free.
    Bitmap = 8,
    /// An allocation starts at a cluster that is not a heap cluster, or
    /// has none although it is not empty.
    InvalidCluster = 9,
    /// A chain links to a free, reserved or out-of-range value.
    BrokenChain = 10,
    /// A chain links back to one of its own clusters.
    CyclicChain = 11,
    /// A cluster is in more than one allocation.
    CrossLink = 12,
    /// Clusters the bitmap marks allocated that no allocation reaches.
    LostClusters = 13,
    /// A name that is empty, holds a character exFAT forbids, or is `.` or
    /// `..`.
    BadName = 14,
    /// `VolumeFlags` marks the volume dirty.
    Dirty = 15,
    /// `PercentInUse` is neither unknown nor the share of allocated
    /// clusters.
    PercentInUse = 16,
    /// FAT entries 0 and 1 do not hold the media type and `0xFFFFFFFF`.
    FatEntries = 17,
    /// An Allocation Bitmap, Up-case Table or Volume Label entry outside
    /// the root directory, a second one in it, or a label that is too long.
    RootEntry = 18,
    /// A File entry without its Stream Extension and File Name entries, or
    /// secondary entries that follow no primary entry.
    EntrySet = 19,
    /// A directory whose size is not a whole number of clusters, above
    /// 256 MiB, or different from its `ValidDataLength`.
    DirectorySize = 20,
    /// A chain runs into a cluster marked bad.
    BadCluster = 21,
    /// An allocation is longer or shorter than its `DataLength` needs.
    SizeMismatch = 22,
    /// A directory nested deeper than the checker follows.
    TooDeep = 23,
    /// The checker stopped walking the tree: the allocations it reached,
    /// counted again each time a cross-linked directory is entered, add up
    /// to more than twice the volume's clusters.
    LimitExceeded = 24,
}

impl Detail {
    const ALL: [Self; 24] = [
        Self::BootSector,
        Self::BootChecksum,
        Self::BackupBootRegion,
        Self::UpcaseTable,
        Self::SetChecksum,
        Self::NameHash,
        Self::ValidDataLength,
        Self::Bitmap,
        Self::InvalidCluster,
        Self::BrokenChain,
        Self::CyclicChain,
        Self::CrossLink,
        Self::LostClusters,
        Self::BadName,
        Self::Dirty,
        Self::PercentInUse,
        Self::FatEntries,
        Self::RootEntry,
        Self::EntrySet,
        Self::DirectorySize,
        Self::BadCluster,
        Self::SizeMismatch,
        Self::TooDeep,
        Self::LimitExceeded,
    ];

    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::BootSector => "invalid boot sector",
            Self::BootChecksum => "boot region checksum does not match",
            Self::BackupBootRegion => "backup boot region differs from the main one",
            Self::UpcaseTable => "invalid up-case table",
            Self::SetChecksum => "entry set checksum does not match",
            Self::NameHash => "name hash does not match the name",
            Self::ValidDataLength => "valid data length exceeds the data length",
            Self::Bitmap => "allocation bitmap is missing or wrong",
            Self::InvalidCluster => "allocation starts at an invalid cluster",
            Self::BrokenChain => "chain links to a free, reserved or out-of-range cluster",
            Self::CyclicChain => "chain links back to itself",
            Self::CrossLink => "cluster is in more than one allocation",
            Self::LostClusters => "allocated clusters no allocation reaches",
            Self::BadName => "invalid name",
            Self::Dirty => "volume is marked dirty",
            Self::PercentInUse => "PercentInUse is wrong",
            Self::FatEntries => "FAT entries 0 and 1 are not the media type and end marks",
            Self::RootEntry => "misplaced or repeated root directory entry",
            Self::EntrySet => "incomplete entry set",
            Self::DirectorySize => "invalid directory size",
            Self::BadCluster => "chain runs into a bad cluster",
            Self::SizeMismatch => "allocation length does not match the data length",
            Self::TooDeep => "directory nested too deep to check",
            Self::LimitExceeded => "tree is too cross-linked to check; the walk stopped",
        }
    }

    /// The detail an exFAT operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of these codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-fat::exfat`
    /// domain.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn error<E>(self, kind: ErrorKind) -> Error<E> {
        Error::new(kind, self.description()).with_detail(self.code())
    }

    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn corrupt<E>(self) -> Error<E> {
        self.error(ErrorKind::Corrupt)
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_round_trip_through_their_codes() {
        for (index, detail) in Detail::ALL.into_iter().enumerate() {
            assert_eq!(detail as usize, index + 1);
            let err: Error<()> = Error::new(ErrorKind::Corrupt, "").with_detail(detail.code());
            assert_eq!(Detail::of(&err), Some(detail));
        }
        let fat = crate::Detail::BootSector.code();
        assert_eq!(Detail::from_code(fat), None);
        assert_eq!(crate::Detail::from_code(Detail::BootSector.code()), None);
    }

    #[test]
    fn detail_codes_are_pinned() {
        let pinned = [
            (Detail::BootSector, 1),
            (Detail::BootChecksum, 2),
            (Detail::BackupBootRegion, 3),
            (Detail::UpcaseTable, 4),
            (Detail::SetChecksum, 5),
            (Detail::NameHash, 6),
            (Detail::ValidDataLength, 7),
            (Detail::Bitmap, 8),
            (Detail::InvalidCluster, 9),
            (Detail::BrokenChain, 10),
            (Detail::CyclicChain, 11),
            (Detail::CrossLink, 12),
            (Detail::LostClusters, 13),
            (Detail::BadName, 14),
            (Detail::Dirty, 15),
            (Detail::PercentInUse, 16),
            (Detail::FatEntries, 17),
            (Detail::RootEntry, 18),
            (Detail::EntrySet, 19),
            (Detail::DirectorySize, 20),
            (Detail::BadCluster, 21),
            (Detail::SizeMismatch, 22),
            (Detail::TooDeep, 23),
            (Detail::LimitExceeded, 24),
        ];
        assert_eq!(pinned.len(), Detail::ALL.len());
        for (detail, number) in pinned {
            let code = DetailCode::new("hadris-fat::exfat", number);
            assert_eq!(detail.code(), code);
            assert_eq!(Detail::from_code(code), Some(detail));
        }
    }
}

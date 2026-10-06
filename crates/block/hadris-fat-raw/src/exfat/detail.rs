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
    BootChecksum,
    /// The backup boot region differs from the main one.
    BackupBootRegion,
    /// The up-case table is missing, does not match its checksum, or maps
    /// one of the first 128 code points other than the specification
    /// requires.
    UpcaseTable,
    /// An entry set whose `SetChecksum` does not match its entries.
    SetChecksum,
    /// A `NameHash` that is not the hash of the up-cased name.
    NameHash,
    /// A file whose `ValidDataLength` exceeds its `DataLength`.
    ValidDataLength,
    /// The Allocation Bitmap is missing, too short, or marks clusters in
    /// use as free.
    Bitmap,
    /// An allocation starts at a cluster that is not a heap cluster, or
    /// has none although it is not empty.
    InvalidCluster,
    /// A chain links to a free, reserved or out-of-range value.
    BrokenChain,
    /// A chain links back to one of its own clusters.
    CyclicChain,
    /// A cluster is in more than one allocation.
    CrossLink,
    /// Clusters the bitmap marks allocated that no allocation reaches.
    LostClusters,
    /// A name that is empty, holds a character exFAT forbids, or is `.` or
    /// `..`.
    BadName,
    /// `VolumeFlags` marks the volume dirty.
    Dirty,
    /// `PercentInUse` is neither unknown nor the share of allocated
    /// clusters.
    PercentInUse,
    /// FAT entries 0 and 1 do not hold the media type and `0xFFFFFFFF`.
    FatEntries,
    /// An Allocation Bitmap, Up-case Table or Volume Label entry outside
    /// the root directory, a second one in it, or a label that is too long.
    RootEntry,
    /// A File entry without its Stream Extension and File Name entries, or
    /// secondary entries that follow no primary entry.
    EntrySet,
    /// A directory whose size is not a whole number of clusters, above
    /// 256 MiB, or different from its `ValidDataLength`.
    DirectorySize,
    /// A chain runs into a cluster marked bad.
    BadCluster,
    /// An allocation is longer or shorter than its `DataLength` needs.
    SizeMismatch,
    /// A directory nested deeper than the checker follows.
    TooDeep,
}

impl Detail {
    const ALL: [Self; 23] = [
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
}

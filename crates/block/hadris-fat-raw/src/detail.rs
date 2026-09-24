use core::fmt;

use hadris_fs::{DetailCode, Error, ErrorKind};

use crate::boot::BootError;

const DOMAIN: &str = "hadris-fat";

/// What exactly is wrong on a FAT12, FAT16 or FAT32 volume, shared by the
/// errors of mounting and reading and by the findings of `check`.
///
/// Read it back from an [`Error`] with [`Detail::of`], or from a finding
/// with [`Detail::from_code`]. Codes never change meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The boot sector is not a valid FAT boot sector, or a field is
    /// inconsistent with the rest of the volume.
    BootSector = 1,
    /// The FAT32 backup boot sector differs from the boot sector.
    BackupBootSector,
    /// FAT entries 0 and 1 do not hold the media byte and end-of-chain
    /// marks.
    ReservedEntries,
    /// The FAT32 FSInfo sector has wrong signatures.
    FsInfo,
    /// The FSInfo free count is neither unknown nor the free clusters in
    /// the FAT.
    FreeCount,
    /// A mirrored FAT copy differs from the active one.
    FatCopiesDiffer,
    /// A chain starts at a cluster that is not a data cluster, or a
    /// directory has no cluster.
    InvalidCluster,
    /// A chain links to a free, reserved or out-of-range value.
    BrokenChain,
    /// A chain links back to one of its own clusters.
    CyclicChain,
    /// A file's chain is longer or shorter than its size needs.
    SizeMismatch,
    /// A cluster is in more than one chain.
    CrossLink,
    /// Allocated clusters that no chain reaches.
    LostClusters,
    /// A `.` or `..` entry is missing, wrong or out of place.
    DotEntries,
    /// A short name holds a byte FAT does not allow.
    BadName,
    /// A long name does not match the checksum of its short entry.
    LfnChecksum,
    /// Long-name fragments that do not end in a short entry.
    OrphanLfn,
    /// The FAT16 or FAT32 clean-shutdown bit is clear: the volume was not
    /// unmounted cleanly.
    Dirty,
    /// A chain runs into a cluster marked bad.
    BadCluster,
    /// A directory entry records a non-zero size.
    DirectorySize,
    /// A volume label entry outside the root directory, or a second one.
    Label,
}

impl Detail {
    const ALL: [Self; 20] = [
        Self::BootSector,
        Self::BackupBootSector,
        Self::ReservedEntries,
        Self::FsInfo,
        Self::FreeCount,
        Self::FatCopiesDiffer,
        Self::InvalidCluster,
        Self::BrokenChain,
        Self::CyclicChain,
        Self::SizeMismatch,
        Self::CrossLink,
        Self::LostClusters,
        Self::DotEntries,
        Self::BadName,
        Self::LfnChecksum,
        Self::OrphanLfn,
        Self::Dirty,
        Self::BadCluster,
        Self::DirectorySize,
        Self::Label,
    ];

    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::BootSector => "invalid boot sector",
            Self::BackupBootSector => "backup boot sector differs from the boot sector",
            Self::ReservedEntries => "FAT entries 0 and 1 are not the media byte and end marks",
            Self::FsInfo => "invalid FSInfo sector",
            Self::FreeCount => "FSInfo free count is wrong",
            Self::FatCopiesDiffer => "FAT copies differ",
            Self::InvalidCluster => "chain starts at an invalid cluster",
            Self::BrokenChain => "chain links to a free, reserved or out-of-range cluster",
            Self::CyclicChain => "chain links back to itself",
            Self::SizeMismatch => "chain length does not match the file size",
            Self::CrossLink => "cluster is in more than one chain",
            Self::LostClusters => "allocated clusters no chain reaches",
            Self::DotEntries => "missing or wrong dot entry",
            Self::BadName => "invalid short name",
            Self::LfnChecksum => "long name checksum does not match its short entry",
            Self::OrphanLfn => "long name fragments without a short entry",
            Self::Dirty => "volume was not unmounted cleanly",
            Self::BadCluster => "chain runs into a bad cluster",
            Self::DirectorySize => "directory entry records a size",
            Self::Label => "misplaced volume label entry",
        }
    }

    /// The detail a FAT operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of these codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-fat` domain.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn corrupt<E>(self) -> Error<E> {
        Error::new(ErrorKind::Corrupt, self.description()).with_detail(self.code())
    }

    /// The error a rejected boot sector fails to mount with.
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn boot<E>(err: BootError) -> Error<E> {
        let message = match err {
            BootError::NotFat(field) | BootError::Corrupt(field) => field,
            BootError::Signature(_) => "boot sector signature is not 0xAA55",
            BootError::RootCluster { .. } => "FAT32 root cluster is not a data cluster",
            BootError::FsInfoSignature { field, .. } => field,
        };
        Error::new(err.kind(), message).with_detail(Self::BootSector.code())
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
        let foreign = DetailCode::new("hadris-iso", Detail::BootSector as u16);
        assert_eq!(Detail::from_code(foreign), None);
    }

    #[test]
    fn boot_errors_carry_the_boot_sector_detail() {
        let err: Error<()> = Detail::boot(BootError::NotFat("bytes per sector"));
        assert_eq!(err.kind(), ErrorKind::NotRecognized);
        assert_eq!(Detail::of(&err), Some(Detail::BootSector));
    }
}

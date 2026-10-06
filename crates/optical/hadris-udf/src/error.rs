use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};

const DOMAIN: &str = "hadris-udf";

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure or
/// option to report. Read it back from an [`Error`] with [`Detail::of`],
/// or from a [`PathError`](hadris_fs::PathError) with
/// [`Detail::from_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The Volume Recognition Sequence has no NSR descriptor inside an
    /// extended area: the device holds no UDF volume. The kind is
    /// [`ErrorKind::NotRecognized`].
    RecognitionSequence = 1,
    /// No Anchor Volume Descriptor Pointer is valid.
    Anchor = 2,
    /// Neither volume descriptor sequence holds a primary, a logical volume
    /// and a partition descriptor.
    DescriptorSequence = 3,
    /// A descriptor tag is wrong: its checksum, CRC, identifier, version or
    /// location.
    Descriptor = 4,
    /// The logical block size is not a power of two from 512 to 4096, it
    /// disagrees with the anchor, or the device block is larger.
    BlockSize = 5,
    /// A partition map other than type 1 (virtual, sparable or metadata
    /// partitions), or, when writing, a revision that needs one: UDF 2.50
    /// and later require a metadata partition.
    PartitionMap = 6,
    /// A partition reference names no partition, or an extent runs past its
    /// partition.
    Partition = 7,
    /// The File Set Descriptor is invalid.
    FileSet = 8,
    /// A File Entry or Extended File Entry is invalid.
    Icb = 9,
    /// A list of allocation descriptors is invalid, too long, or ends
    /// before the file.
    AllocationDescriptor = 10,
    /// A File Identifier Descriptor is invalid.
    FileIdentifier = 11,
    /// A symbolic link's path components are invalid.
    PathComponent = 12,
    /// A structure lies outside the device.
    OutsideImage = 13,
    /// A volume identifier does not fit its field.
    Identifier = 14,
    /// The image would exceed the 32-bit block numbers of UDF.
    ImageTooLarge = 15,
    /// A time lies outside the years 1 to 9999 a timestamp holds.
    Timestamp = 16,
    /// A file of the tree changed or vanished between measuring and
    /// writing.
    Content = 17,
    /// A file's content is extents on the device: a standalone volume
    /// cannot use them, and in a bridge volume they must be whole aligned
    /// blocks after the UDF structures.
    StoredContent = 18,
    /// The output device's block size does not divide 2048.
    OutputBlockSize = 19,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::RecognitionSequence => "no NSR descriptor in the volume recognition sequence",
            Self::Anchor => "no valid anchor volume descriptor pointer",
            Self::DescriptorSequence => "incomplete volume descriptor sequence",
            Self::Descriptor => "invalid descriptor tag",
            Self::BlockSize => "unsupported logical or device block size",
            Self::PartitionMap => "unsupported partition map",
            Self::Partition => "a partition reference or extent is out of bounds",
            Self::FileSet => "invalid file set descriptor",
            Self::Icb => "invalid file entry",
            Self::AllocationDescriptor => "invalid allocation descriptors",
            Self::FileIdentifier => "invalid file identifier descriptor",
            Self::PathComponent => "invalid symbolic link path components",
            Self::OutsideImage => "a structure lies outside the device",
            Self::Identifier => "volume identifier too long",
            Self::ImageTooLarge => "image exceeds 32-bit block numbers",
            Self::Timestamp => "time outside the years 1 to 9999",
            Self::Content => "a file changed after it was measured",
            Self::StoredContent => "stored content does not fit the volume",
            Self::OutputBlockSize => "output block size does not divide 2048",
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl Detail {
    const ALL: [Self; 19] = [
        Self::RecognitionSequence,
        Self::Anchor,
        Self::DescriptorSequence,
        Self::Descriptor,
        Self::BlockSize,
        Self::PartitionMap,
        Self::Partition,
        Self::FileSet,
        Self::Icb,
        Self::AllocationDescriptor,
        Self::FileIdentifier,
        Self::PathComponent,
        Self::OutsideImage,
        Self::Identifier,
        Self::ImageTooLarge,
        Self::Timestamp,
        Self::Content,
        Self::StoredContent,
        Self::OutputBlockSize,
    ];

    /// The detail a UDF operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-udf` domain.
    /// Codes never change meaning.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    pub(crate) fn error<E>(self, kind: ErrorKind) -> Error<E> {
        Error::new(kind, self.description()).with_detail(self.code())
    }

    pub(crate) fn corrupt<E>(self) -> Error<E> {
        self.error(ErrorKind::Corrupt)
    }

    pub(crate) fn invalid<E>(self) -> Error<E> {
        self.error(ErrorKind::InvalidInput)
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
        for detail in Detail::ALL {
            let err: Error<()> = Error::new(ErrorKind::Corrupt, "").with_detail(detail.code());
            assert_eq!(Detail::of(&err), Some(detail));
        }
        let foreign = DetailCode::new("another-crate", Detail::ALL[0] as u16);
        assert_eq!(Detail::from_code(foreign), None);
    }
}

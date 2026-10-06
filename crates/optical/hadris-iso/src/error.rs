use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};

const DOMAIN: &str = "hadris-iso";

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure or
/// option to report. Read it back from an [`Error`] with [`Detail::of`],
/// or from a [`PathError`](hadris_fs::PathError) with
/// [`Detail::from_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// A volume descriptor lacks `CD001` or has an unknown version. At the
    /// first descriptor the kind is [`ErrorKind::NotRecognized`].
    DescriptorHeader = 1,
    /// The descriptor set has no primary volume descriptor, or no
    /// terminator within 64 descriptors.
    NoPrimaryDescriptor = 2,
    /// The set terminator's body is not zero.
    Terminator = 3,
    /// The logical block size is not a power of two from 512 to 2048, the
    /// device block is larger than 4096 bytes, or a descriptor's redundant
    /// fields disagree.
    BlockSize = 4,
    /// A descriptor's both-endian fields disagree.
    DescriptorFields = 5,
    /// A directory record breaks ECMA-119: its length, padding, redundant
    /// fields, or a pointer outside the image.
    DirectoryRecord = 6,
    /// A multi-extent file's records are cut short or do not match.
    MultiExtent = 7,
    /// A descriptor or record points outside the image.
    OutsideImage = 8,
    /// A file is interleaved or lives on another volume of a set.
    Interleaved = 9,
    /// A system use area or a Rock Ridge entry is malformed.
    SystemUse = 10,
    /// The El Torito boot catalog is malformed.
    BootCatalog = 11,
    /// The image has no tree for the requested namespace.
    NoNamespace = 12,
    /// A boot image named in the options is not a file in the tree, a
    /// diskette image is not the diskette's size, or a load size is zero.
    BootImage = 13,
    /// A boot image is too small for the requested boot information table.
    BootInfoTable = 14,
    /// The boot catalog path clashes with an entry of the tree or has no
    /// parent directory.
    CatalogPath = 15,
    /// The Rock Ridge relocation directory's name is taken by a root entry
    /// that is not a directory, a root directory named `rr_moved` or
    /// `.rr_moved` would come before it, or relocation is refused and the
    /// tree is too deep.
    Relocation = 16,
    /// A volume identifier does not fit its field.
    Identifier = 17,
    /// The image would exceed the 32-bit block counts of ISO 9660 or an MBR
    /// partition entry.
    ImageTooLarge = 18,
    /// The hybrid boot options do not fit the image: the EFI partition image
    /// is missing or outside the ISO area, or the boot code is too long.
    HybridBoot = 19,
    /// A file of the tree changed or vanished between measuring and
    /// writing.
    Content = 20,
    /// A file's content is extents on a device other than the output.
    StoredContent = 21,
    /// The output device's block size does not divide 2048.
    OutputBlockSize = 22,
    /// A session cannot be appended or rewritten: the image is not the one
    /// the session was opened on, or its layout leaves no room.
    Session = 23,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::DescriptorHeader => "invalid volume descriptor header",
            Self::NoPrimaryDescriptor => "no primary volume descriptor",
            Self::Terminator => "invalid volume descriptor set terminator",
            Self::BlockSize => "unsupported logical or device block size",
            Self::DescriptorFields => "volume descriptor fields disagree",
            Self::DirectoryRecord => "invalid directory record",
            Self::MultiExtent => "invalid multi-extent file",
            Self::OutsideImage => "a pointer lies outside the image",
            Self::Interleaved => "interleaved or multi-volume file",
            Self::SystemUse => "invalid system use area",
            Self::BootCatalog => "invalid El Torito boot catalog",
            Self::NoNamespace => "the image has no tree for this namespace",
            Self::BootImage => "boot image missing or unfit for its entry",
            Self::BootInfoTable => "boot image too small for a boot information table",
            Self::CatalogPath => "invalid boot catalog path",
            Self::Relocation => "directory tree too deep or relocation directory clashes",
            Self::Identifier => "volume identifier too long",
            Self::ImageTooLarge => "image exceeds a 32-bit block count",
            Self::HybridBoot => "hybrid boot options do not fit the image",
            Self::Content => "a file changed after it was measured",
            Self::StoredContent => "stored content lives on another device",
            Self::OutputBlockSize => "output block size does not divide 2048",
            Self::Session => "session cannot be written on this image",
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl Detail {
    const ALL: [Self; 23] = [
        Self::DescriptorHeader,
        Self::NoPrimaryDescriptor,
        Self::Terminator,
        Self::BlockSize,
        Self::DescriptorFields,
        Self::DirectoryRecord,
        Self::MultiExtent,
        Self::OutsideImage,
        Self::Interleaved,
        Self::SystemUse,
        Self::BootCatalog,
        Self::NoNamespace,
        Self::BootImage,
        Self::BootInfoTable,
        Self::CatalogPath,
        Self::Relocation,
        Self::Identifier,
        Self::ImageTooLarge,
        Self::HybridBoot,
        Self::Content,
        Self::StoredContent,
        Self::OutputBlockSize,
        Self::Session,
    ];

    /// The detail an ISO 9660 operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-iso` domain.
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

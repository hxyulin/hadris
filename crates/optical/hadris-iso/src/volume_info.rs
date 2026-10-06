use hadris_fs::DateTime;

use crate::raw::{DecDateTime, PrimaryVolumeDescriptor};

/// A text identifier of the volume descriptors.
///
/// Joliet and ISO 9660:1999 descriptors carry the same values. The `File`
/// identifiers name files of the root directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsoId {
    /// The system that may use the system area, up to 32 bytes.
    System,
    /// The volume name, up to 32 bytes; `CDROM` by default.
    Volume,
    /// The volume set, up to 128 bytes.
    VolumeSet,
    /// The publisher, up to 128 bytes.
    Publisher,
    /// The data preparer, up to 128 bytes.
    Preparer,
    /// The application, up to 128 bytes; `HADRIS-ISO` by default.
    Application,
    /// The copyright file, up to 37 bytes.
    CopyrightFile,
    /// The abstract file, up to 37 bytes.
    AbstractFile,
    /// The bibliographic file, up to 37 bytes.
    BibliographicFile,
}

impl IsoId {
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// A date of the volume descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IsoDate {
    /// When the volume was created; the options' time by default.
    Created,
    /// When the volume was last modified; the options' time by default.
    Modified,
    /// When the data becomes obsolete; unspecified by default.
    Expires,
    /// When the data may first be used; unspecified by default.
    Effective,
}

impl IsoDate {
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// What the primary volume descriptor records, as `IsoFs::info` returns
/// it.
///
/// Identifiers are the stored bytes without their trailing spaces, since
/// many producers store bytes that are not ASCII.
#[derive(Clone, Copy)]
pub struct VolumeInfo {
    block_size: u32,
    volume_space_size: u32,
    system: [u8; 32],
    volume: [u8; 32],
    volume_set: [u8; 128],
    publisher: [u8; 128],
    preparer: [u8; 128],
    application: [u8; 128],
    copyright: [u8; 37],
    abstract_file: [u8; 37],
    bibliographic: [u8; 37],
    dates: [DecDateTime; 4],
}

impl VolumeInfo {
    pub(crate) fn new(pvd: &PrimaryVolumeDescriptor) -> Self {
        Self {
            block_size: u32::from(pvd.logical_block_size.get()),
            volume_space_size: pvd.volume_space_size.get(),
            system: *pvd.system_identifier.as_bytes(),
            volume: *pvd.volume_identifier.as_bytes(),
            volume_set: *pvd.volume_set_identifier.as_bytes(),
            publisher: *pvd.publisher_identifier.as_bytes(),
            preparer: *pvd.preparer_identifier.as_bytes(),
            application: *pvd.application_identifier.as_bytes(),
            copyright: *pvd.copyright_file_identifier.as_bytes(),
            abstract_file: *pvd.abstract_file_identifier.as_bytes(),
            bibliographic: *pvd.bibliographic_file_identifier.as_bytes(),
            dates: [
                pvd.creation_date,
                pvd.modification_date,
                pvd.expiration_date,
                pvd.effective_date,
            ],
        }
    }

    /// The logical block size in bytes.
    pub const fn block_size(&self) -> u32 {
        self.block_size
    }

    /// The number of logical blocks the volume declares.
    pub const fn volume_space_size(&self) -> u32 {
        self.volume_space_size
    }

    /// The identifier `id` as stored, without trailing spaces.
    pub fn id(&self, id: IsoId) -> &[u8] {
        let bytes: &[u8] = match id {
            IsoId::System => &self.system,
            IsoId::Volume => &self.volume,
            IsoId::VolumeSet => &self.volume_set,
            IsoId::Publisher => &self.publisher,
            IsoId::Preparer => &self.preparer,
            IsoId::Application => &self.application,
            IsoId::CopyrightFile => &self.copyright,
            IsoId::AbstractFile => &self.abstract_file,
            IsoId::BibliographicFile => &self.bibliographic,
        };
        let end = bytes
            .iter()
            .rposition(|&byte| byte != b' ')
            .map_or(0, |pos| pos + 1);
        &bytes[..end]
    }

    /// The date `date`, or `None` when it is not specified or not valid.
    pub fn date(&self, date: IsoDate) -> Option<DateTime> {
        self.dates[date.index()].to_datetime()
    }
}

impl core::fmt::Debug for VolumeInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VolumeInfo")
            .field("block_size", &self.block_size)
            .field("volume_space_size", &self.volume_space_size)
            .field("volume", &core::str::from_utf8(self.id(IsoId::Volume)))
            .finish_non_exhaustive()
    }
}

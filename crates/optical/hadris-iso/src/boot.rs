use crate::raw::{self, BootSectionEntry};

/// The platform of an El Torito boot entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Platform {
    /// 80x86 BIOS, `0x00`.
    X86,
    /// PowerPC, `0x01`.
    PowerPc,
    /// Mac, `0x02`.
    Mac,
    /// UEFI, `0xEF`.
    Efi,
    /// Any other platform id.
    Other(u8),
}

impl Platform {
    /// The platform for an id byte.
    pub const fn from_id(id: u8) -> Self {
        match id {
            0x00 => Self::X86,
            0x01 => Self::PowerPc,
            0x02 => Self::Mac,
            0xEF => Self::Efi,
            other => Self::Other(other),
        }
    }

    /// The id byte.
    pub const fn id(self) -> u8 {
        match self {
            Self::X86 => 0x00,
            Self::PowerPc => 0x01,
            Self::Mac => 0x02,
            Self::Efi => 0xEF,
            Self::Other(id) => id,
        }
    }
}

/// How firmware presents an El Torito boot image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Emulation {
    /// The image is loaded as it is; `load_size` 512-byte sectors of it.
    NoEmulation,
    /// A 1.2 MB diskette.
    Floppy12,
    /// A 1.44 MB diskette.
    Floppy144,
    /// A 2.88 MB diskette.
    Floppy288,
    /// A hard disk, as drive 0x80.
    HardDisk,
}

impl Emulation {
    /// The emulation named by the low nibble of a boot media type byte.
    pub const fn from_media_type(media_type: u8) -> Option<Self> {
        Some(match media_type & 0x0F {
            0 => Self::NoEmulation,
            1 => Self::Floppy12,
            2 => Self::Floppy144,
            3 => Self::Floppy288,
            4 => Self::HardDisk,
            _ => return None,
        })
    }

    /// The media type byte.
    pub const fn media_type(self) -> u8 {
        match self {
            Self::NoEmulation => 0,
            Self::Floppy12 => 1,
            Self::Floppy144 => 2,
            Self::Floppy288 => 3,
            Self::HardDisk => 4,
        }
    }
}

/// One boot entry of a [`BootCatalog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    raw: BootSectionEntry,
    platform: Platform,
    section: Option<usize>,
}

impl CatalogEntry {
    /// The platform: the validation entry's for the default entry, the
    /// section header's for the others.
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// The emulation, or `None` for a media type the specification does not
    /// define.
    pub const fn emulation(&self) -> Option<Emulation> {
        Emulation::from_media_type(self.raw.boot_media_type)
    }

    /// Whether the boot indicator says the entry is bootable.
    pub const fn is_bootable(&self) -> bool {
        self.raw.boot_indicator == raw::BOOTABLE
    }

    /// The logical block of the boot image.
    pub const fn load_block(&self) -> u32 {
        self.raw.load_rba.get()
    }

    /// The number of 512-byte virtual sectors firmware loads.
    pub const fn sector_count(&self) -> u16 {
        self.raw.sector_count.get()
    }

    /// The real-mode load segment; 0 means the default 0x7C0.
    pub const fn load_segment(&self) -> u16 {
        self.raw.load_segment.get()
    }

    /// The index of the section the entry belongs to, or `None` for the
    /// default entry.
    pub const fn section(&self) -> Option<usize> {
        self.section
    }

    /// The entry as stored.
    pub const fn raw(&self) -> &BootSectionEntry {
        &self.raw
    }
}

/// An El Torito boot catalog in a caller's buffer, as `IsoFs::boot_catalog`
/// reads and checks it. Its entries are parsed as they are iterated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootCatalog<'b> {
    block: u32,
    /// The checked catalog, from the validation entry to its last entry.
    bytes: &'b [u8],
}

impl<'b> BootCatalog<'b> {
    pub(crate) const fn new(block: u32, bytes: &'b [u8]) -> Self {
        Self { block, bytes }
    }

    /// The logical block the catalog starts at.
    pub const fn block(&self) -> u32 {
        self.block
    }

    /// The platform in the validation entry.
    pub const fn platform(&self) -> Platform {
        Platform::from_id(self.bytes[1])
    }

    /// The manufacturer string of the validation entry, without trailing
    /// spaces and NULs.
    pub fn id_string(&self) -> &'b [u8] {
        let id = &self.bytes[4..28];
        let end = id
            .iter()
            .rposition(|&byte| byte != 0 && byte != b' ')
            .map_or(0, |pos| pos + 1);
        &id[..end]
    }

    /// The default entry, then every section entry in catalog order.
    pub fn entries(&self) -> CatalogEntries<'b> {
        CatalogEntries {
            parser: CatalogParser::new(),
            bytes: self.bytes,
            pos: 0,
        }
    }

    /// The default (initial) entry.
    pub fn default_entry(&self) -> CatalogEntry {
        let raw: BootSectionEntry = bytemuck::pod_read_unaligned(&self.bytes[32..64]);
        CatalogEntry {
            raw,
            platform: self.platform(),
            section: None,
        }
    }

    /// The catalog as stored, from the validation entry to its last entry.
    pub const fn as_bytes(&self) -> &'b [u8] {
        self.bytes
    }
}

/// The entries of a [`BootCatalog`], parsed one at a time.
#[derive(Clone)]
pub struct CatalogEntries<'b> {
    parser: CatalogParser,
    bytes: &'b [u8],
    pos: usize,
}

impl core::fmt::Debug for CatalogEntries<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CatalogEntries")
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

impl Iterator for CatalogEntries<'_> {
    type Item = CatalogEntry;

    fn next(&mut self) -> Option<CatalogEntry> {
        while let Some(chunk) = self.bytes.get(self.pos..self.pos + 32) {
            self.pos += 32;
            let mut entry = [0u8; 32];
            entry.copy_from_slice(chunk);
            match self.parser.feed(&entry) {
                Ok(Step::Entry(entry)) => return Some(entry),
                Ok(Step::More) => {}
                _ => break,
            }
        }
        self.pos = self.bytes.len();
        None
    }
}

impl core::iter::FusedIterator for CatalogEntries<'_> {}

/// Parses a catalog 32 bytes at a time.
#[derive(Clone)]
pub(crate) struct CatalogParser {
    validation: Option<raw::BootValidationEntry>,
    state: State,
}

#[derive(Clone, Copy)]
enum State {
    Validation,
    Default,
    Header {
        section: usize,
    },
    Entries {
        left: u16,
        platform: u8,
        section: usize,
        last: bool,
    },
    Done,
}

/// What the parser found in a 32-byte entry.
pub(crate) enum Step {
    Entry(CatalogEntry),
    More,
    Done,
}

impl CatalogParser {
    /// The most entries read before the catalog counts as malformed.
    pub(crate) const MAX_ENTRIES: usize = 1024;

    pub(crate) fn new() -> Self {
        Self {
            validation: None,
            state: State::Validation,
        }
    }

    pub(crate) fn feed(&mut self, chunk: &[u8; 32]) -> Result<Step, ()> {
        match self.state {
            State::Validation => {
                let validation: raw::BootValidationEntry = bytemuck::cast(*chunk);
                if !validation.is_valid() {
                    return Err(());
                }
                self.validation = Some(validation);
                self.state = State::Default;
                Ok(Step::More)
            }
            State::Default => {
                let platform = self.validation.map_or(0, |v| v.platform_id);
                self.state = State::Header { section: 0 };
                Ok(Step::Entry(CatalogEntry {
                    raw: bytemuck::cast(*chunk),
                    platform: Platform::from_id(platform),
                    section: None,
                }))
            }
            State::Header { section } => {
                if !matches!(chunk[0], raw::HEADER_MORE | raw::HEADER_FINAL) {
                    self.state = State::Done;
                    return Ok(Step::Done);
                }
                let header: raw::BootCatalogHeader = bytemuck::cast(*chunk);
                let last = header.header_type == raw::HEADER_FINAL;
                self.state = match header.section_count.get() {
                    0 if last => State::Done,
                    0 => State::Header {
                        section: section + 1,
                    },
                    left => State::Entries {
                        left,
                        platform: header.platform_id,
                        section,
                        last,
                    },
                };
                Ok(Step::More)
            }
            State::Entries {
                left,
                platform,
                section,
                last,
            } => {
                if chunk[0] == 0x44 {
                    return Ok(Step::More);
                }
                self.state = match left - 1 {
                    0 if last => State::Done,
                    0 => State::Header {
                        section: section + 1,
                    },
                    left => State::Entries {
                        left,
                        platform,
                        section,
                        last,
                    },
                };
                Ok(Step::Entry(CatalogEntry {
                    raw: bytemuck::cast(*chunk),
                    platform: Platform::from_id(platform),
                    section: Some(section),
                }))
            }
            State::Done => Ok(Step::Done),
        }
    }

    /// Whether the catalog has ended, so no more chunks belong to it.
    pub(crate) fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }
}

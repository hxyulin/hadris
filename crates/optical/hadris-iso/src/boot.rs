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
pub struct BootCatalogEntry {
    raw: BootSectionEntry,
    platform: Platform,
    section: Option<usize>,
}

impl BootCatalogEntry {
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

/// A parsed El Torito boot catalog, as `IsoFs::boot_catalog` reads it.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootCatalog {
    block: u32,
    validation: raw::BootValidationEntry,
    entries: alloc::vec::Vec<BootCatalogEntry>,
}

#[cfg(feature = "alloc")]
impl BootCatalog {
    /// The logical block the catalog starts at.
    pub const fn block(&self) -> u32 {
        self.block
    }

    /// The platform in the validation entry.
    pub const fn platform(&self) -> Platform {
        Platform::from_id(self.validation.platform_id)
    }

    /// The manufacturer string of the validation entry.
    pub fn id_string(&self) -> &[u8] {
        let end = self
            .validation
            .id_string
            .iter()
            .rposition(|&byte| byte != 0 && byte != b' ')
            .map_or(0, |pos| pos + 1);
        &self.validation.id_string[..end]
    }

    /// The default entry, then every section entry in catalog order.
    pub fn entries(&self) -> &[BootCatalogEntry] {
        &self.entries
    }

    /// The default (initial) entry.
    pub fn default_entry(&self) -> &BootCatalogEntry {
        &self.entries[0]
    }

    /// The validation entry as stored.
    pub const fn validation(&self) -> &raw::BootValidationEntry {
        &self.validation
    }
}

/// Parses a catalog 32 bytes at a time.
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
    Entry(BootCatalogEntry),
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
                Ok(Step::Entry(BootCatalogEntry {
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
                Ok(Step::Entry(BootCatalogEntry {
                    raw: bytemuck::cast(*chunk),
                    platform: Platform::from_id(platform),
                    section: Some(section),
                }))
            }
            State::Done => Ok(Step::Done),
        }
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn finish(
        self,
        block: u32,
        entries: alloc::vec::Vec<BootCatalogEntry>,
    ) -> Option<BootCatalog> {
        if entries.is_empty() {
            return None;
        }
        Some(BootCatalog {
            block,
            validation: self.validation?,
            entries,
        })
    }
}

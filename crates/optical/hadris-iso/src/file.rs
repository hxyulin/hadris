use hadris_common::types::file::FixedFilename;

#[cfg(feature = "alloc")]
use crate::joliet::JolietLevel;

#[cfg(feature = "write")]
use crate::types::{Charset, CharsetD, CharsetD1};

/// The type of directory entry, indicating the ISO interchange level and features
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    Level1 {
        supports_lowercase: bool,
        supports_rrip: bool,
    },
    Level2 {
        supports_lowercase: bool,
        supports_rrip: bool,
    },
    Level3 {
        supports_lowercase: bool,
        supports_rrip: bool,
    },
    #[cfg(feature = "alloc")]
    Joliet {
        level: JolietLevel,
        supports_rrip: bool,
    },
}

impl Default for EntryType {
    fn default() -> Self {
        Self::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        }
    }
}

impl EntryType {
    // Usefulness coefficient:
    // bits 0-3 = base level (lowercase = 4,5,6 Joliet = level 12, 13, 14)
    // bit 4 = rrip
    pub fn supports_rrip(&self) -> bool {
        match self {
            Self::Level1 { supports_rrip, .. } => *supports_rrip,
            Self::Level2 { supports_rrip, .. } => *supports_rrip,
            Self::Level3 { supports_rrip, .. } => *supports_rrip,
            #[cfg(feature = "alloc")]
            Self::Joliet { supports_rrip, .. } => *supports_rrip,
        }
    }

    // Usefulness coefficient:
    // bits 0-3 = base level (lowercase = 4,5,6 Joliet = level 12, 13, 14)
    // bit 4 = rrip
    fn usefulness(self) -> u8 {
        match self {
            Self::Level1 {
                supports_lowercase,
                supports_rrip,
            } => (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            Self::Level2 {
                supports_lowercase,
                supports_rrip,
            } => 0x01 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            Self::Level3 {
                supports_lowercase,
                supports_rrip,
            } => 0x02 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            #[cfg(feature = "alloc")]
            Self::Joliet {
                level,
                supports_rrip,
            } => (level as u8 + 11) | (supports_rrip as u8) << 4,
        }
    }
}

impl Ord for EntryType {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.usefulness().cmp(&other.usefulness())
    }
}

impl PartialOrd for EntryType {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "alloc")]
impl From<JolietLevel> for EntryType {
    fn from(value: JolietLevel) -> Self {
        Self::Joliet {
            level: value,
            supports_rrip: false,
        }
    }
}

pub type FilenameL1 = FixedFilename<14>;
pub type FilenameL2 = FixedFilename<32>;
pub type FilenameL3 = FixedFilename<207>;

#[cfg(feature = "write")]
pub enum ConvertedName {
    Level1(FilenameL1),
    Level2(FilenameL2),
    Level3(FilenameL3),
    Joliet(FixedFilename<207>),
}

#[cfg(feature = "write")]
impl ConvertedName {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Level1(name) => name.as_bytes(),
            Self::Level2(name) => name.as_bytes(),
            Self::Level3(name) => name.as_bytes(),
            Self::Joliet(name) => name.as_bytes(),
        }
    }
}

#[cfg(feature = "write")]
impl EntryType {
    pub fn convert_name(self, name: &str) -> ConvertedName {
        match self {
            Self::Level1 {
                supports_lowercase, ..
            } => ConvertedName::Level1(convert_l1(name, supports_lowercase)),
            Self::Level2 {
                supports_lowercase, ..
            } => ConvertedName::Level2(convert_l2(name, supports_lowercase)),
            Self::Level3 {
                supports_lowercase, ..
            } => ConvertedName::Level3(convert_l3(name, supports_lowercase)),
            Self::Joliet { level, .. } => match level {
                // All Joliet levels use UTF-16 BE encoding
                JolietLevel::Level1 | JolietLevel::Level2 | JolietLevel::Level3 => {
                    ConvertedName::Joliet(convert_joliet3(name))
                }
            },
        }
    }
}

#[cfg(feature = "write")]
pub fn convert_l1(name: &str, supports_lowercase: bool) -> FixedFilename<14> {
    let mut l1 = FixedFilename::empty();
    let name_bytes = name.as_bytes();
    match name.find('.') {
        Some(index) => {
            // We copy the basename, at most 8 bytes
            let basename = l1.push_slice(&name_bytes[0..index.min(8)]);
            let basename = l1.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
            // Extension length excluding the dot character
            let ext_len = (name.len() - index - 1).min(3);
            l1.push_byte(b'.');
            let ext = l1.push_slice(&name_bytes[index + 1..(index + 1 + ext_len).min(name.len())]);
            let ext = l1.data[ext].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(ext);
            } else {
                CharsetD::substitute_invalid(ext);
            }
        }
        None => {
            let len = name.len().min(8);
            let basename = l1.push_slice(&name_bytes[0..len]);
            let basename = l1.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l1.push_slice(b";1");
    l1
}

#[cfg(feature = "write")]
pub fn convert_l2(name: &str, supports_lowercase: bool) -> FilenameL2 {
    let mut l2 = FilenameL2::empty();
    let name_bytes = name.as_bytes();
    // Max: 30 bytes for name (reserve 2 for ";1")
    const MAX_NAME_LEN: usize = 30;

    match name.find('.') {
        Some(index) => {
            let basename_end = index.min(MAX_NAME_LEN);
            let basename = l2.push_slice(&name_bytes[0..basename_end]);
            let basename = l2.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }

            // Calculate remaining space for extension (subtract basename length and 1 for dot)
            let remaining = MAX_NAME_LEN.saturating_sub(basename_end + 1);
            if remaining > 0 {
                l2.push_byte(b'.');
                let ext_end = (index + 1 + remaining).min(name.len());
                let ext = l2.push_slice(&name_bytes[index + 1..ext_end]);
                let ext = l2.data[ext].iter_mut();
                if supports_lowercase {
                    CharsetD1::substitute_invalid(ext);
                } else {
                    CharsetD::substitute_invalid(ext);
                }
            }
        }
        None => {
            let len = name.len().min(MAX_NAME_LEN);
            let basename = l2.push_slice(&name_bytes[0..len]);
            let basename = l2.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l2.push_slice(b";1");
    l2
}

#[cfg(feature = "write")]
pub fn convert_l3(name: &str, supports_lowercase: bool) -> FilenameL3 {
    let mut l3 = FilenameL3::empty();
    let name_bytes = name.as_bytes();
    // Max: 207 bytes for name (no version suffix in L3)
    const MAX_NAME_LEN: usize = 207;

    match name.find('.') {
        Some(index) => {
            let basename_end = index.min(MAX_NAME_LEN);
            let basename = l3.push_slice(&name_bytes[0..basename_end]);
            let basename = l3.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }

            // Calculate remaining space for extension (subtract basename length and 1 for dot)
            let remaining = MAX_NAME_LEN.saturating_sub(basename_end + 1);
            if remaining > 0 {
                l3.push_byte(b'.');
                let ext_end = (index + 1 + remaining).min(name.len());
                let ext = l3.push_slice(&name_bytes[index + 1..ext_end]);
                let ext = l3.data[ext].iter_mut();
                if supports_lowercase {
                    CharsetD1::substitute_invalid(ext);
                } else {
                    CharsetD::substitute_invalid(ext);
                }
            }
        }
        None => {
            let len = name.len().min(MAX_NAME_LEN);
            let basename = l3.push_slice(&name_bytes[0..len]);
            let basename = l3.data[basename].iter_mut();
            if supports_lowercase {
                CharsetD1::substitute_invalid(basename);
            } else {
                CharsetD::substitute_invalid(basename);
            }
        }
    }
    l3
}

#[cfg(feature = "write")]
pub fn convert_joliet3(name: &str) -> FixedFilename<207> {
    let mut j1 = FixedFilename::empty();
    for (written, c) in name.encode_utf16().enumerate() {
        if written >= 206 / 2 {
            // We reached the maximum we can write
            break;
        }
        let bytes = c.to_be_bytes();
        j1.push_slice(&bytes);
    }

    j1
}

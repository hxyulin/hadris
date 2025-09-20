use hadris_common::types::file::FixedFilename;

use crate::joliet::JolietLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord)]
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
    fn usefulness(self) -> u8 {
        match self {
            Self::Level1 {
                supports_lowercase,
                supports_rrip,
            } => 0x00 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            Self::Level2 {
                supports_lowercase,
                supports_rrip,
            } => 0x01 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            Self::Level3 {
                supports_lowercase,
                supports_rrip,
            } => 0x02 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
            Self::Joliet {
                level,
                supports_rrip,
            } => (level as u8 + 11) | (supports_rrip as u8) << 4,
        }
    }
}

impl PartialOrd for EntryType {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        PartialOrd::partial_cmp(&self.usefulness(), &other.usefulness())
    }
}

pub type FilenameL1 = FixedFilename<14>;
pub type FilenameL2 = FixedFilename<32>;
pub type FilenameL3 = FixedFilename<207>;

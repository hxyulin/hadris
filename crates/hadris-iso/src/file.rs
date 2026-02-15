use hadris_common::types::file::FixedFilename;

#[cfg(feature = "alloc")]
use crate::joliet::JolietLevel;

/// The type of directory entry, indicating the ISO interchange level and features
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
            } => 0x00 | (supports_lowercase as u8) << 2 | (supports_rrip as u8) << 4,
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

impl PartialOrd for EntryType {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        PartialOrd::partial_cmp(&self.usefulness(), &other.usefulness())
    }
}

pub type FilenameL1 = FixedFilename<14>;
pub type FilenameL2 = FixedFilename<32>;
pub type FilenameL3 = FixedFilename<207>;

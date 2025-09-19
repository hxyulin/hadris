use alloc::string::String;

use crate::{boot::options::BootOptions, joliet::JolietLevel, read::PathSeparator};

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub volume_name: String,
    pub sector_size: usize,
    pub features: CreationFeatures,
    pub path_seperator: PathSeparator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseIsoLevel {
    /// L1 Filenames
    /// Supports only uppercase and useing the 8.3 format
    Level1 {
        supports_lowercase: bool,
        supports_rrip: bool,
    },
    /// L2 Filenames
    /// Supports up to 30 characters
    Level2 {
        supports_lowercase: bool,
        supports_rrip: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CreationFeatures {
    /// The base Filename Level
    /// This only supports ASCII uppercase, numbers, and '_' for compatibility reasons.
    pub filenames: BaseIsoLevel,
    /// The L3 Filename Levle
    /// This supports filenames up to 207 characters, without using Joliet or Rock Ridge
    pub long_filenames: bool,
    /// The Joliet Extension
    pub joliet: Option<JolietLevel>,
    pub el_torito: Option<BootOptions>,
}

impl Default for CreationFeatures {
    fn default() -> Self {
        Self {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            el_torito: None,
        }
    }
}

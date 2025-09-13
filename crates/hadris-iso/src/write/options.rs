use alloc::string::String;

use crate::read::PathSeparator;

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
    Level1,
    /// L2 Filenames
    /// Supports up to 30 characters
    Level2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JolietLevel {
    Level1,
}

impl JolietLevel {
    pub fn all() -> &'static [JolietLevel] {
        static LEVELS: [JolietLevel; 1] = [
            JolietLevel::Level1,
        ];
        &LEVELS
    }

    pub fn escape_sequence(self) -> [u8; 32] {
        match self {
            Self::Level1 => *b"%/C                             "
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CreationFeatures {
    /// The base Filename Level
    /// This only supports ASCII uppercase, numbers, and '_' for compatibility reasons.
    pub filenames: BaseIsoLevel,
    /// The L3 Filename Levle
    /// This supports filenames up to 207 characters, without using Joliet or Rock Ridge
    pub long_filenames: bool,
    /// The Joliet Extension
    pub joliet: Option<JolietLevel>,
    pub el_torito: Option<ElToritoOptions>,
}

#[derive(Debug, Clone, Copy)]
pub struct ElToritoOptions {

}

impl Default for CreationFeatures {
    fn default() -> Self {
        Self {
            filenames: BaseIsoLevel::Level1,
            long_filenames: false,
            joliet: None,
            el_torito: None,
        } 
    }
}

use alloc::string::String;

use crate::file::FilenameLevel;

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub volume_name: String,
    pub sector_size: usize,
    pub features: CreationFeatures,
}

#[derive(Debug, Clone)]
pub struct CreationFeatures {
    pub filenames: FilenameLevel,
}

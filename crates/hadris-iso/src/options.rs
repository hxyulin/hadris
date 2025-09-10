use alloc::string::String;

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub volume_name: String,
    pub sector_size: usize,
    pub features: CreationFeatures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilenameLevel {
    /// L1 Filenames
    /// Supports only uppercase and useing the 8.3 format
    Level1,
    /// L2 Filenames
    /// Supports up to 30 characters
    Level2,
    /// L3 Filenames
    /// Supports up to 207 characters
    Level3,
}

fn convert_l1(name: &str) -> String {
    todo!()
}

impl FilenameLevel {
    pub fn convert(self, name: &str) -> String {
        match self {
            Self::Level1 => convert_l1(name),
            Self::Level2 => todo!(),
            Self::Level3 => todo!(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreationFeatures {
    pub filenames: FilenameLevel,
}

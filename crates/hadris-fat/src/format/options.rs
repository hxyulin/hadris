//! FAT volume formatting options.
//!
//! This module provides configuration types for formatting FAT12/16/32 volumes.

use crate::FatType;

/// FAT volume formatting options.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Total volume size in bytes
    pub volume_size: u64,
    /// Volume label (up to 11 characters)
    pub volume_label: VolumeLabel,
    /// OEM name (up to 8 characters, default "HADRISFT")
    pub oem_name: OemName,
    /// Sector size (512, 1024, 2048, or 4096 bytes)
    pub sector_size: SectorSize,
    /// Sectors per cluster (auto-calculated if None)
    pub sectors_per_cluster: Option<u8>,
    /// FAT type selection (auto or forced)
    pub fat_type: FatTypeSelection,
    /// Number of FAT copies (1 or 2, default 2)
    pub fat_copies: u8,
    /// Root directory entry count (FAT12/16 only, default 512)
    pub root_entry_count: u16,
    /// Hidden sectors (for partitioned media)
    pub hidden_sectors: u32,
    /// Media type descriptor
    pub media_type: MediaType,
    /// Volume ID (random if None)
    pub volume_id: Option<u32>,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            volume_size: 0,
            volume_label: VolumeLabel::default(),
            oem_name: OemName::default(),
            sector_size: SectorSize::default(),
            sectors_per_cluster: None,
            fat_type: FatTypeSelection::Auto,
            fat_copies: 2,
            root_entry_count: 512,
            hidden_sectors: 0,
            media_type: MediaType::FixedDisk,
            volume_id: None,
        }
    }
}

impl FormatOptions {
    /// Create new format options with the specified volume size.
    pub fn new(volume_size: u64) -> Self {
        Self {
            volume_size,
            ..Default::default()
        }
    }

    /// Set the volume label.
    pub fn with_label(mut self, label: &str) -> Self {
        self.volume_label = VolumeLabel::new(label);
        self
    }

    /// Set the sector size.
    pub fn with_sector_size(mut self, size: SectorSize) -> Self {
        self.sector_size = size;
        self
    }

    /// Set the FAT type selection.
    pub fn with_fat_type(mut self, fat_type: FatTypeSelection) -> Self {
        self.fat_type = fat_type;
        self
    }

    /// Set sectors per cluster.
    pub fn with_sectors_per_cluster(mut self, spc: u8) -> Self {
        self.sectors_per_cluster = Some(spc);
        self
    }

    /// Set the number of FAT copies.
    pub fn with_fat_copies(mut self, copies: u8) -> Self {
        self.fat_copies = copies.clamp(1, 2);
        self
    }

    /// Set the media type.
    pub fn with_media_type(mut self, media_type: MediaType) -> Self {
        self.media_type = media_type;
        self
    }

    /// Set hidden sectors count.
    pub fn with_hidden_sectors(mut self, hidden: u32) -> Self {
        self.hidden_sectors = hidden;
        self
    }

    /// Set the volume ID.
    pub fn with_volume_id(mut self, id: u32) -> Self {
        self.volume_id = Some(id);
        self
    }
}

/// Volume label (11 characters max, space-padded).
#[derive(Debug, Clone)]
pub struct VolumeLabel([u8; 11]);

impl VolumeLabel {
    /// Create a new volume label from a string.
    ///
    /// The string is converted to uppercase, truncated to 11 characters,
    /// and space-padded.
    pub fn new(s: &str) -> Self {
        let mut bytes = [b' '; 11];
        for (i, c) in s.chars().take(11).enumerate() {
            let c = c.to_ascii_uppercase();
            if c.is_ascii() && Self::is_valid_char(c as u8) {
                bytes[i] = c as u8;
            } else {
                bytes[i] = b'_';
            }
        }
        Self(bytes)
    }

    /// Create a "NO NAME" volume label.
    pub fn no_name() -> Self {
        Self(*b"NO NAME    ")
    }

    /// Check if a character is valid for volume labels.
    fn is_valid_char(c: u8) -> bool {
        matches!(c, b'A'..=b'Z' | b'0'..=b'9' | b' ' | b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'(' | b')' | b'-' | b'@' | b'^' | b'_' | b'`' | b'{' | b'}' | b'~')
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 11] {
        &self.0
    }
}

impl Default for VolumeLabel {
    fn default() -> Self {
        Self::no_name()
    }
}

/// OEM name (8 characters max, space-padded).
#[derive(Debug, Clone)]
pub struct OemName([u8; 8]);

impl OemName {
    /// Create a new OEM name from a string.
    pub fn new(s: &str) -> Self {
        let mut bytes = [b' '; 8];
        for (i, c) in s.chars().take(8).enumerate() {
            if c.is_ascii() {
                bytes[i] = c as u8;
            }
        }
        Self(bytes)
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

impl Default for OemName {
    fn default() -> Self {
        Self(*b"HADRISFT")
    }
}

/// Sector size options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorSize {
    /// 512 bytes per sector (most common)
    S512 = 512,
    /// 1024 bytes per sector
    S1024 = 1024,
    /// 2048 bytes per sector
    S2048 = 2048,
    /// 4096 bytes per sector
    S4096 = 4096,
}

impl SectorSize {
    /// Get the size in bytes.
    pub fn bytes(self) -> usize {
        self as usize
    }
}

impl Default for SectorSize {
    fn default() -> Self {
        Self::S512
    }
}

impl TryFrom<usize> for SectorSize {
    type Error = ();

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            512 => Ok(Self::S512),
            1024 => Ok(Self::S1024),
            2048 => Ok(Self::S2048),
            4096 => Ok(Self::S4096),
            _ => Err(()),
        }
    }
}

/// FAT type selection for formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FatTypeSelection {
    /// Automatically select based on volume size
    #[default]
    Auto,
    /// Force FAT12 (volumes < ~16 MB)
    Fat12,
    /// Force FAT16 (volumes < ~2 GB)
    Fat16,
    /// Force FAT32 (volumes >= ~32 MB)
    Fat32,
}

impl FatTypeSelection {
    /// Convert to FatType if not Auto.
    pub fn as_fat_type(self) -> Option<FatType> {
        match self {
            Self::Auto => None,
            Self::Fat12 => Some(FatType::Fat12),
            Self::Fat16 => Some(FatType::Fat16),
            Self::Fat32 => Some(FatType::Fat32),
        }
    }
}

impl From<FatType> for FatTypeSelection {
    fn from(fat_type: FatType) -> Self {
        match fat_type {
            FatType::Fat12 => Self::Fat12,
            FatType::Fat16 => Self::Fat16,
            FatType::Fat32 => Self::Fat32,
        }
    }
}

/// Media type descriptor byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Fixed disk (0xF8)
    FixedDisk,
    /// Removable media (0xF0)
    Removable,
    /// Custom media type byte
    Custom(u8),
}

impl MediaType {
    /// Get the media type byte value.
    pub fn value(self) -> u8 {
        match self {
            Self::FixedDisk => 0xF8,
            Self::Removable => 0xF0,
            Self::Custom(v) => v,
        }
    }
}

impl Default for MediaType {
    fn default() -> Self {
        Self::FixedDisk
    }
}

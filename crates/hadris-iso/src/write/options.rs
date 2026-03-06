use alloc::string::String;

use super::super::boot::options::BootOptions;
use super::super::read::PathSeparator;
use super::super::rrip::RripOptions;
use crate::joliet::JolietLevel;

/// Hybrid boot options for creating bootable ISO images from USB/disk.
///
/// This enables the ISO to be bootable when written directly to a USB drive
/// or other storage media, in addition to being bootable as a CD/DVD.
#[derive(Debug, Clone, Default)]
pub struct HybridBootOptions {
    /// The type of partition table to write.
    pub partition_scheme: PartitionScheme,
    /// Optional MBR bootstrap code to inject (must be 446 bytes or less).
    /// This is typically the first stage of a bootloader like GRUB or Syslinux.
    pub mbr_bootstrap: Option<alloc::vec::Vec<u8>>,
    /// Whether to mark the ISO partition as bootable in the MBR.
    pub bootable: bool,
}

impl HybridBootOptions {
    /// Create options for MBR-only hybrid boot (BIOS systems).
    pub fn mbr() -> Self {
        Self {
            partition_scheme: PartitionScheme::Mbr,
            mbr_bootstrap: None,
            bootable: true,
        }
    }

    /// Create options for GPT-only boot (UEFI systems).
    pub fn gpt() -> Self {
        Self {
            partition_scheme: PartitionScheme::Gpt,
            mbr_bootstrap: None,
            bootable: false,
        }
    }

    /// Create options for hybrid MBR+GPT boot (dual BIOS/UEFI systems).
    pub fn hybrid() -> Self {
        Self {
            partition_scheme: PartitionScheme::Hybrid,
            mbr_bootstrap: None,
            bootable: true,
        }
    }

    /// Set the MBR bootstrap code.
    pub fn with_bootstrap(mut self, bootstrap: alloc::vec::Vec<u8>) -> Self {
        self.mbr_bootstrap = Some(bootstrap);
        self
    }
}

/// The partition scheme to use for hybrid boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartitionScheme {
    /// No partition table (CD/DVD only, not USB bootable).
    #[default]
    None,
    /// MBR partition table only (for BIOS USB boot).
    Mbr,
    /// GPT partition table only (for UEFI boot).
    Gpt,
    /// Hybrid MBR + GPT (for dual BIOS/UEFI boot).
    /// Creates a protective MBR with GPT, plus MBR entries mirroring key partitions.
    Hybrid,
}

#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub volume_name: String,
    pub system_id: Option<String>,
    pub volume_set_id: Option<String>,
    pub publisher_id: Option<String>,
    pub preparer_id: Option<String>,
    pub application_id: Option<String>,
    pub sector_size: usize,
    pub features: CreationFeatures,
    pub path_separator: PathSeparator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseIsoLevel {
    /// L1 Filenames
    /// Supports only uppercase and using the 8.3 format
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
    /// The L3 Filename Level
    /// This supports filenames up to 207 characters, without using Joliet or Rock Ridge
    pub long_filenames: bool,
    /// The Joliet Extension for Unicode filenames
    pub joliet: Option<JolietLevel>,
    /// Rock Ridge extension options for POSIX filesystem semantics
    pub rock_ridge: Option<RripOptions>,
    /// El-Torito boot options (for CD/DVD boot)
    pub el_torito: Option<BootOptions>,
    /// Hybrid boot options (for USB/disk boot)
    /// Enables the ISO to be bootable when written directly to a USB drive.
    pub hybrid_boot: Option<HybridBootOptions>,
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
            rock_ridge: None,
            el_torito: None,
            hybrid_boot: None,
        }
    }
}

impl CreationFeatures {
    /// Create features with Rock Ridge enabled (default settings)
    pub fn with_rock_ridge() -> Self {
        Self {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: true,
            },
            rock_ridge: Some(RripOptions::default()),
            ..Default::default()
        }
    }

    /// Create features with Joliet enabled
    pub fn with_joliet(level: JolietLevel) -> Self {
        Self {
            joliet: Some(level),
            ..Default::default()
        }
    }

    /// Create features with both Rock Ridge and Joliet enabled
    pub fn with_extensions() -> Self {
        Self {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: true,
            },
            joliet: Some(JolietLevel::Level3),
            rock_ridge: Some(RripOptions::default()),
            ..Default::default()
        }
    }

    /// Create features with hybrid boot enabled (MBR for USB boot)
    pub fn with_hybrid_boot(scheme: PartitionScheme) -> Self {
        Self {
            hybrid_boot: Some(HybridBootOptions {
                partition_scheme: scheme,
                mbr_bootstrap: None,
                bootable: true,
            }),
            ..Default::default()
        }
    }
}

impl From<BaseIsoLevel> for crate::file::EntryType {
    fn from(value: BaseIsoLevel) -> Self {
        match value {
            BaseIsoLevel::Level1 {
                supports_lowercase,
                supports_rrip,
            } => Self::Level1 {
                supports_lowercase,
                supports_rrip,
            },
            BaseIsoLevel::Level2 {
                supports_lowercase,
                supports_rrip,
            } => Self::Level2 {
                supports_lowercase,
                supports_rrip,
            },
        }
    }
}

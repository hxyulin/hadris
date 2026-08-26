use alloc::string::String;
use alloc::vec::Vec;

use super::super::boot::options::BootOptions;
use super::super::read::PathSeparator;
use super::super::rrip::RripOptions;
use crate::file::EntryType;
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
    /// Path (in the ISO tree, using the configured path separator) of the
    /// El Torito UEFI boot image to additionally expose as a GPT EFI System
    /// Partition.
    ///
    /// Applies to [`PartitionScheme::Gpt`] and [`PartitionScheme::Hybrid`].
    /// When `None`, and the El Torito options contain exactly one
    /// `PlatformId::UEFI` section entry, that entry's boot image is exposed
    /// automatically. Formatting fails with a not-found error when the
    /// configured path does not resolve to a file in the ISO tree.
    pub efi_boot_partition: Option<String>,
}

impl HybridBootOptions {
    /// Create options for MBR-only hybrid boot (BIOS systems).
    pub fn mbr() -> Self {
        Self {
            partition_scheme: PartitionScheme::Mbr,
            mbr_bootstrap: None,
            bootable: true,
            efi_boot_partition: None,
        }
    }

    /// Create options for GPT-only boot (UEFI systems).
    pub fn gpt() -> Self {
        Self {
            partition_scheme: PartitionScheme::Gpt,
            mbr_bootstrap: None,
            bootable: false,
            efi_boot_partition: None,
        }
    }

    /// Create options for hybrid MBR+GPT boot (dual BIOS/UEFI systems).
    pub fn hybrid() -> Self {
        Self {
            partition_scheme: PartitionScheme::Hybrid,
            mbr_bootstrap: None,
            bootable: true,
            efi_boot_partition: None,
        }
    }

    /// Set the MBR bootstrap code.
    pub fn bootstrap(mut self, bootstrap: alloc::vec::Vec<u8>) -> Self {
        self.mbr_bootstrap = Some(bootstrap);
        self
    }

    /// Set the path of the El Torito UEFI boot image to expose as a GPT EFI
    /// System Partition. See [`HybridBootOptions::efi_boot_partition`].
    pub fn with_efi_boot_partition(mut self, path: impl Into<String>) -> Self {
        self.efi_boot_partition = Some(path.into());
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
/// Represents IsoFormatOptions.
pub struct IsoFormatOptions {
    /// The `volume_name` field.
    pub volume_name: String,
    /// The `system_id` field.
    pub system_id: Option<String>,
    /// The `volume_set_id` field.
    pub volume_set_id: Option<String>,
    /// The `publisher_id` field.
    pub publisher_id: Option<String>,
    /// The `preparer_id` field.
    pub preparer_id: Option<String>,
    /// The `application_id` field.
    pub application_id: Option<String>,
    /// The `sector_size` field.
    pub sector_size: usize,
    /// The `features` field.
    pub features: CreationFeatures,
    /// The `path_separator` field.
    pub path_separator: PathSeparator,
    /// When false (default), PVD string fields are stored as-is without charset
    /// validation (matching xorriso/genisoimage behavior). When true, auto-converts
    /// lowercase to uppercase and substitutes invalid characters for ECMA-119 compliance.
    pub strict_charset: bool,
}

impl IsoFormatOptions {
    /// Returns the partition scheme for hybrid boot.
    pub fn partition_scheme(&self) -> Option<PartitionScheme> {
        self.features
            .hybrid_boot
            .as_ref()
            .map(|h| h.partition_scheme)
    }

    /// Returns true if Rock Ridge is enabled and deep directory relocation is active.
    pub fn has_rock_ridge_deep_dirs(&self) -> bool {
        self.features.rock_ridge.is_some_and(|options| options.enabled && options.relocate_deep_dirs)
    }

    /// Builds the list of entry types (ISO levels, Joliet, etc.) to write.
    ///
    /// Order: PVD base -> Level3 (if long filenames) -> Joliet (if enabled).
    pub fn entry_types(&self) -> Vec<EntryType> {
        let mut entry_types = Vec::new();
        entry_types.push(self.features.filenames.into());

        if self.features.long_filenames {
            entry_types.push(EntryType::Level3 {
                supports_lowercase: true,
                supports_rrip: false,
            });
        }

        if let Some(joliet) = self.features.joliet {
            entry_types.push(joliet.into());
        }

        entry_types
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Identifies a BaseIsoLevel value.
pub enum BaseIsoLevel {
    /// L1 Filenames
    /// Supports only uppercase and using the 8.3 format
    Level1 {
        /// The `supports_lowercase` field.
        supports_lowercase: bool,
        /// The `supports_rrip` field.
        supports_rrip: bool,
    },
    /// L2 Filenames
    /// Supports up to 30 characters
    Level2 {
        /// The `supports_lowercase` field.
        supports_lowercase: bool,
        /// The `supports_rrip` field.
        supports_rrip: bool,
    },
    /// ISO 9660 interchange level 3.
    ///
    /// Level 3 retains the Level 2 filename rules and permits a logical file
    /// to be represented by multiple consecutive directory records/extents.
    Level3 {
        /// Whether lowercase ASCII names are accepted.
        supports_lowercase: bool,
        /// Whether Rock Ridge system-use fields are emitted.
        supports_rrip: bool,
    },
}

#[derive(Debug, Clone)]
/// Represents CreationFeatures.
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
    pub fn rock_ridge() -> Self {
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
    pub fn joliet(level: JolietLevel) -> Self {
        Self {
            joliet: Some(level),
            ..Default::default()
        }
    }

    /// Create features with both Rock Ridge and Joliet enabled
    pub fn extensions() -> Self {
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
    pub fn hybrid_boot(scheme: PartitionScheme) -> Self {
        Self {
            hybrid_boot: Some(HybridBootOptions {
                partition_scheme: scheme,
                mbr_bootstrap: None,
                bootable: true,
                efi_boot_partition: None,
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
            BaseIsoLevel::Level3 {
                supports_lowercase,
                supports_rrip,
            } => Self::Level2 {
                supports_lowercase,
                supports_rrip,
            },
        }
    }
}

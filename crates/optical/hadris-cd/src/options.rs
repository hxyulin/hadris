//! Configuration options for hybrid CD/DVD image creation

use hadris_iso::boot::options::BootOptions;
use hadris_iso::joliet::JolietLevel;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{BaseIsoLevel, HybridBootOptions};
use hadris_udf::UdfRevision;

/// Options for creating a hybrid ISO+UDF image
#[derive(Debug, Clone)]
pub struct CdOptions {
    /// Volume identifier (used by both ISO and UDF)
    pub volume_id: String,
    /// Sector size (almost always 2048)
    pub sector_size: usize,
    /// ISO 9660 options
    pub iso: IsoOptions,
    /// UDF options
    pub udf: UdfOptions,
    /// El-Torito boot options
    pub boot: Option<BootOptions>,
    /// Hybrid boot options (MBR/GPT for USB booting)
    pub hybrid_boot: Option<HybridBootOptions>,
}

impl Default for CdOptions {
    fn default() -> Self {
        Self {
            volume_id: String::from("CDROM"),
            sector_size: 2048,
            iso: IsoOptions::default(),
            udf: UdfOptions::default(),
            boot: None,
            hybrid_boot: None,
        }
    }
}

impl CdOptions {
    /// Create options with a volume ID
    pub fn with_volume_id(volume_id: impl Into<String>) -> Self {
        Self {
            volume_id: volume_id.into(),
            ..Default::default()
        }
    }

    /// Set the volume ID
    pub fn volume_id(mut self, id: impl Into<String>) -> Self {
        self.volume_id = id.into();
        self
    }

    /// Enable Joliet support (Windows long filenames)
    pub fn with_joliet(mut self) -> Self {
        self.iso.joliet = Some(JolietLevel::Level3);
        self
    }

    /// Enable Rock Ridge support (POSIX filenames and permissions)
    pub fn with_rock_ridge(mut self) -> Self {
        self.iso.rock_ridge = Some(RripOptions::default());
        self
    }

    /// Set boot options
    pub fn with_boot(mut self, boot: BootOptions) -> Self {
        self.boot = Some(boot);
        self
    }

    /// Set hybrid boot options (for USB booting)
    pub fn with_hybrid_boot(mut self, hybrid: HybridBootOptions) -> Self {
        self.hybrid_boot = Some(hybrid);
        self
    }

    /// Disable UDF (create ISO-only image)
    pub fn iso_only(mut self) -> Self {
        self.udf.enabled = false;
        self
    }

    /// Disable ISO (create UDF-only image)
    pub fn udf_only(mut self) -> Self {
        self.iso.enabled = false;
        self
    }
}

/// ISO 9660 specific options
#[derive(Debug, Clone)]
pub struct IsoOptions {
    /// Enable ISO 9660 (default: true)
    pub enabled: bool,
    /// Base ISO level (L1 = 8.3, L2 = 30 chars)
    pub level: BaseIsoLevel,
    /// Enable ISO 9660:1999 (Level 3, long filenames)
    pub long_filenames: bool,
    /// Joliet extension (Windows long filenames)
    pub joliet: Option<JolietLevel>,
    /// Rock Ridge extension (POSIX attributes)
    pub rock_ridge: Option<RripOptions>,
}

impl Default for IsoOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            level: BaseIsoLevel::Level2 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: true,
            joliet: Some(JolietLevel::Level3),
            rock_ridge: None,
        }
    }
}

/// UDF specific options
#[derive(Debug, Clone)]
pub struct UdfOptions {
    /// Enable UDF (default: true)
    pub enabled: bool,
    /// UDF revision
    pub revision: UdfRevision,
}

impl Default for UdfOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            revision: UdfRevision::V1_02,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let opts = CdOptions::default();
        assert_eq!(opts.volume_id, "CDROM");
        assert_eq!(opts.sector_size, 2048);
        assert!(opts.iso.enabled);
        assert!(opts.udf.enabled);
    }

    #[test]
    fn test_builder_pattern() {
        let opts = CdOptions::with_volume_id("MY_DISC")
            .with_joliet()
            .with_rock_ridge();

        assert_eq!(opts.volume_id, "MY_DISC");
        assert!(opts.iso.joliet.is_some());
        assert!(opts.iso.rock_ridge.is_some());
    }
}

//! Configuration options for hybrid CD/DVD image creation

pub use hadris_iso::JolietLevel;
use hadris_iso::{ElTorito, HybridBoot, IsoLevel, NameCase, RockRidge};
use hadris_udf::UdfRevision;

/// Options for creating a hybrid ISO+UDF image
#[derive(Debug, Clone)]
pub struct OpticalImageOptions {
    /// Volume identifier (used by both ISO and UDF)
    pub volume_id: String,
    /// Sector size (almost always 2048)
    pub sector_size: usize,
    /// ISO 9660 options
    pub iso: IsoOptions,
    /// UDF options
    pub udf: UdfOptions,
    /// El-Torito boot options
    pub boot: Option<ElTorito>,
    /// Hybrid boot options (MBR/GPT for USB booting)
    pub hybrid_boot: Option<HybridBoot>,
}

impl Default for OpticalImageOptions {
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

impl OpticalImageOptions {
    /// Set the volume ID
    pub fn volume_id(mut self, id: impl Into<String>) -> Self {
        self.volume_id = id.into();
        self
    }

    /// Set the Joliet level used for Windows-compatible long filenames.
    pub fn joliet(mut self, level: JolietLevel) -> Self {
        self.iso.joliet = Some(level);
        self
    }

    /// Set Rock Ridge options.
    pub fn rock_ridge(mut self, options: RockRidge) -> Self {
        self.iso.rock_ridge = Some(options);
        self
    }

    /// Set boot options.
    pub fn boot(mut self, boot: ElTorito) -> Self {
        self.boot = Some(boot);
        self
    }

    /// Set hybrid boot options for USB booting.
    pub fn hybrid_boot(mut self, hybrid: HybridBoot) -> Self {
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
    /// Interchange level of the primary tree (L1 = 8.3, L2 and L3 = 30 chars)
    pub level: IsoLevel,
    /// Whether primary names keep lowercase letters
    pub name_case: NameCase,
    /// Enable the ISO 9660:1999 enhanced tree (long filenames)
    pub long_filenames: bool,
    /// Joliet extension (Windows long filenames)
    pub joliet: Option<JolietLevel>,
    /// Rock Ridge extension (POSIX attributes)
    pub rock_ridge: Option<RockRidge>,
}

impl Default for IsoOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            level: IsoLevel::L2,
            name_case: NameCase::Upper,
            long_filenames: true,
            joliet: Some(JolietLevel::L3),
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
        let opts = OpticalImageOptions::default();
        assert_eq!(opts.volume_id, "CDROM");
        assert_eq!(opts.sector_size, 2048);
        assert!(opts.iso.enabled);
        assert!(opts.udf.enabled);
    }

    #[test]
    fn test_builder_pattern() {
        let opts = OpticalImageOptions::default()
            .volume_id("MY_DISC")
            .joliet(JolietLevel::L3)
            .rock_ridge(RockRidge::default());

        assert_eq!(opts.volume_id, "MY_DISC");
        assert!(opts.iso.joliet.is_some());
        assert!(opts.iso.rock_ridge.is_some());
    }
}

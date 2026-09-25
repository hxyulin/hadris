use hadris_fs::DateTime;
use hadris_iso::{IsoId, IsoLevel, IsoOptions};
use hadris_udf::{UdfId, UdfOptions};

/// Options for a hybrid image: the ISO 9660 and the UDF options.
///
/// The defaults name both volumes `CDROM` and write a Level 2 primary
/// tree with Joliet level 3 and an ISO 9660:1999 enhanced tree beside a
/// UDF 1.02 volume, dated 1980-01-01.
///
/// The writer decides where the ISO 9660 structures start and how long the
/// UDF volume is: it raises [`IsoOptions::with_min_blocks`] to leave room
/// for the UDF metadata, and it sets [`UdfOptions::with_min_blocks`]
/// itself.
///
/// ```rust
/// use hadris_cd::iso::IsoId;
/// use hadris_cd::udf::{UdfId, UdfRevision};
/// use hadris_cd::{CdOptions, IsoOptions, UdfOptions};
///
/// let options = CdOptions::default()
///     .with_iso(
///         IsoOptions::default()
///             .with_id(IsoId::Volume, "MY_DISC")
///             .with_joliet()
///             .with_rock_ridge(),
///     )
///     .with_udf(UdfOptions::default().with_id(UdfId::Volume, "MY_DISC").with_revision(UdfRevision::V2_01));
/// assert_eq!(options.udf().id(UdfId::Volume), Some("MY_DISC"));
/// ```
#[derive(Debug, Clone)]
pub struct CdOptions {
    iso: IsoOptions,
    udf: UdfOptions,
}

impl Default for CdOptions {
    fn default() -> Self {
        Self {
            iso: IsoOptions::default()
                .with_id(IsoId::Volume, "CDROM")
                .with_level(IsoLevel::L2)
                .with_joliet()
                .with_iso1999(),
            udf: UdfOptions::default().with_id(UdfId::Volume, "CDROM"),
        }
    }
}

impl CdOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the ISO 9660 options.
    pub fn with_iso(self, iso: IsoOptions) -> Self {
        Self { iso, ..self }
    }

    /// Replaces the UDF options.
    pub fn with_udf(self, udf: UdfOptions) -> Self {
        Self { udf, ..self }
    }

    /// Sets the time that dates both volumes and the entries without
    /// times.
    pub fn with_time(self, time: DateTime) -> Self {
        Self {
            iso: self.iso.with_time(time),
            udf: self.udf.with_time(time),
        }
    }

    /// The ISO 9660 options.
    pub fn iso(&self) -> &IsoOptions {
        &self.iso
    }

    /// The UDF options.
    pub fn udf(&self) -> &UdfOptions {
        &self.udf
    }
}

use hadris_fs::{Clock, NoClock};
use hadris_iso::{IsoLevel, IsoOptions, JolietLevel, VolumeIdentifiers};
use hadris_udf::UdfOptions;

/// Options for a hybrid image: the ISO 9660 and the UDF options.
///
/// The defaults name both volumes `CDROM` and write a Level 2 primary
/// tree with Joliet level 3 and an ISO 9660:1999 enhanced tree beside a
/// UDF 1.02 volume, dated by [`NoClock`].
///
/// The writer decides where the ISO 9660 structures start and how long the
/// UDF volume is: it raises [`IsoOptions::with_min_blocks`] to leave room
/// for the UDF metadata, and it sets [`UdfOptions::with_bridge`] and
/// [`UdfOptions::with_min_blocks`] itself.
///
/// ```rust
/// use hadris_cd::iso::{JolietLevel, RockRidge, VolumeIdentifiers};
/// use hadris_cd::udf::UdfRevision;
/// use hadris_cd::{CdOptions, IsoOptions, UdfOptions};
///
/// let options = CdOptions::default()
///     .with_iso(
///         IsoOptions::default()
///             .with_volume(VolumeIdentifiers::new("MY_DISC"))
///             .with_joliet(JolietLevel::L3)
///             .with_rock_ridge(RockRidge::default()),
///     )
///     .with_udf(UdfOptions::default().with_volume_id("MY_DISC").with_revision(UdfRevision::V2_01));
/// assert_eq!(options.udf().volume_id(), "MY_DISC");
/// ```
#[derive(Debug, Clone)]
pub struct CdOptions<C = NoClock> {
    iso: IsoOptions<C>,
    udf: UdfOptions<C>,
}

impl Default for CdOptions {
    fn default() -> Self {
        Self {
            iso: IsoOptions::default()
                .with_volume(VolumeIdentifiers::new("CDROM"))
                .with_level(IsoLevel::L2)
                .with_joliet(JolietLevel::L3)
                .with_enhanced_tree(),
            udf: UdfOptions::default().with_volume_id("CDROM"),
        }
    }
}

impl CdOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<C: Clock + Clone> CdOptions<C> {
    /// Replaces the ISO 9660 options.
    pub fn with_iso(self, iso: IsoOptions<C>) -> Self {
        Self { iso, ..self }
    }

    /// Replaces the UDF options.
    pub fn with_udf(self, udf: UdfOptions<C>) -> Self {
        Self { udf, ..self }
    }

    /// Sets the clock that dates both volumes and the entries without
    /// times.
    pub fn with_clock<C2: Clock + Clone>(self, clock: C2) -> CdOptions<C2> {
        CdOptions {
            iso: self.iso.with_clock(clock.clone()),
            udf: self.udf.with_clock(clock),
        }
    }

    /// The ISO 9660 options.
    pub fn iso(&self) -> &IsoOptions<C> {
        &self.iso
    }

    /// The UDF options.
    pub fn udf(&self) -> &UdfOptions<C> {
        &self.udf
    }
}

use crate::volume::Geometry;

/// Geometry and identity read from the boot sector when an NTFS volume mounts.
///
/// Returned by `NtfsFs::info` in either mode without reading the device.
/// The snapshot remains usable after the filesystem is unmounted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VolumeInfo {
    serial: u64,
    sector_size: u32,
    cluster_size: u64,
    total_sectors: u64,
    mft_record_size: u32,
    index_record_size: u32,
}

impl VolumeInfo {
    pub(crate) const fn new(geometry: Geometry) -> Self {
        Self {
            serial: geometry.serial,
            sector_size: geometry.sector_size,
            cluster_size: geometry.cluster_size,
            total_sectors: geometry.total_sectors,
            mft_record_size: geometry.mft_record_size as u32,
            index_record_size: geometry.index_record_size as u32,
        }
    }

    /// The volume serial number stored in the boot sector.
    pub const fn volume_serial(&self) -> u64 {
        self.serial
    }

    /// Bytes per NTFS sector, independently of the device's logical block size.
    pub const fn sector_size(&self) -> u32 {
        self.sector_size
    }

    /// Bytes per allocation cluster.
    pub const fn cluster_size(&self) -> u64 {
        self.cluster_size
    }

    /// Number of sectors declared by the boot sector.
    pub const fn total_sectors(&self) -> u64 {
        self.total_sectors
    }

    /// Bytes per master file table record.
    pub const fn mft_record_size(&self) -> u32 {
        self.mft_record_size
    }

    /// Bytes per directory index record.
    pub const fn index_record_size(&self) -> u32 {
        self.index_record_size
    }
}

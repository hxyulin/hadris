io_transform! {

use core::{cell::Cell, fmt};

use spin::Mutex;

use hadris_common::types::endian::Endian;

use crate::error::{FatError, Result};
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};
use super::dir::{FatDir, FileEntry};
use super::fat_table::{Fat, Fat12, Fat16, Fat32, FatType};
use super::io::{Cluster, ClusterLike, Read, ReadExt, Sector, SectorCursor, SectorLike, Seek, SeekFrom};
use super::read::FileReader;

/// Volume metadata from the boot sector.
///
/// This struct contains information about the volume such as the OEM name,
/// volume serial number, volume label, and filesystem type string.
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// OEM name (8 bytes, space-padded)
    oem_name: [u8; 8],
    /// Volume serial number (4 bytes)
    volume_id: u32,
    /// Volume label (11 bytes, space-padded)
    volume_label: [u8; 11],
    /// Filesystem type string (8 bytes, space-padded)
    fs_type_str: [u8; 8],
}

impl VolumeInfo {
    /// Get the OEM name as a trimmed string.
    pub fn oem_name(&self) -> &str {
        core::str::from_utf8(&self.oem_name)
            .unwrap_or("")
            .trim_end()
    }

    /// Get the volume serial number.
    pub fn volume_id(&self) -> u32 {
        self.volume_id
    }

    /// Get the volume label as a trimmed string.
    pub fn volume_label(&self) -> &str {
        core::str::from_utf8(&self.volume_label)
            .unwrap_or("")
            .trim_end()
    }

    /// Get the filesystem type string as a trimmed string.
    ///
    /// Note: This is informational only and should not be used to determine
    /// the actual FAT type. Use [`FatFs::fat_type()`] instead.
    pub fn fs_type_str(&self) -> &str {
        core::str::from_utf8(&self.fs_type_str)
            .unwrap_or("")
            .trim_end()
    }

    /// Get the raw OEM name bytes.
    pub fn oem_name_raw(&self) -> &[u8; 8] {
        &self.oem_name
    }

    /// Get the raw volume label bytes.
    pub fn volume_label_raw(&self) -> &[u8; 11] {
        &self.volume_label
    }

    /// Get the raw filesystem type string bytes.
    pub fn fs_type_str_raw(&self) -> &[u8; 8] {
        &self.fs_type_str
    }
}

#[derive(Debug)]
pub(crate) struct FatInfo {
    #[allow(dead_code)]
    pub(crate) cluster_size: usize,
    pub(crate) data_start: usize,
    #[allow(dead_code)]
    pub(crate) max_cluster: u32,
}

/// Extension info for FAT12/16 filesystems (fixed root directory)
#[derive(Debug)]
pub(crate) struct Fat12_16FsExt {
    /// Root directory start byte offset
    root_dir_start: usize,
    /// Root directory size in bytes
    root_dir_size: usize,
    /// Number of root directory entries
    #[allow(dead_code)]
    root_entry_count: u16,
}

#[derive(Debug)]
pub(crate) enum FatFsExt {
    Fat12_16(Fat12_16FsExt),
    Fat32(Fat32FsExt),
}

impl FatFsExt {
    /// Get fixed root directory info for FAT12/16
    #[allow(dead_code)]
    fn fixed_root_dir(&self) -> Option<(usize, usize)> {
        match self {
            Self::Fat12_16(ext) => Some((ext.root_dir_start, ext.root_dir_size)),
            Self::Fat32(_) => None,
        }
    }
}

/// Extension info for FAT32 filesystems.
///
/// Uses `Cell` for `free_count` and `next_free` to allow updating the FSInfo
/// sector without requiring mutable access to the entire FatFs.
pub(crate) struct Fat32FsExt {
    /// Sector number of the FSInfo structure
    pub(crate) fs_info_sec: Sector<u16>,
    /// Root directory cluster
    root_clus: Cluster<u32>,
    /// Number of free clusters (from FSInfo, may be stale)
    pub(crate) free_count: Cell<u32>,
    /// Hint for next free cluster (from FSInfo)
    pub(crate) next_free: Cell<Cluster<u32>>,
}

impl fmt::Debug for Fat32FsExt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fat32FsExt")
            .field("fs_info_sec", &self.fs_info_sec)
            .field("root_clus", &self.root_clus)
            .field("free_count", &self.free_count.get())
            .field("next_free", &self.next_free.get())
            .finish()
    }
}

pub struct FatFs<DATA: Seek> {
    pub(crate) data: Mutex<SectorCursor<DATA>>,
    pub(crate) info: FatInfo,
    pub(crate) fat: Fat,
    pub(crate) ext: FatFsExt,
    volume_info: VolumeInfo,
    /// Clock used to stamp newly-created or modified directory entries.
    /// Defaults to [`crate::time::DEFAULT_TIME_PROVIDER`].
    time_provider: &'static dyn crate::time::TimeProvider,
    /// Codepage converter used for short-name encoding/decoding.
    /// Defaults to [`crate::oem::DEFAULT_OEM_CONVERTER`].
    oem_converter: &'static dyn crate::oem::OemCpConverter,
    /// Optional FAT-sector LRU cache. Installed by the builder via
    /// [`FatFsBuilder::with_fat_cache`]; `None` means uncached behaviour
    /// identical to pre-cache versions of this crate. The cache itself
    /// is sync-only and lives behind a [`spin::Mutex`] so it can be
    /// shared between read paths and write paths.
    #[cfg(feature = "cache")]
    pub(crate) fat_cache: Option<Mutex<crate::cache::FatSectorCache>>,
}

impl<DATA: Seek> fmt::Debug for FatFs<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("info", &self.info)
            .field("ext", &self.ext)
            .field("time_provider", &self.time_provider)
            .field("oem_converter", &self.oem_converter)
            .finish_non_exhaustive()
    }
}

/// Builder for [`FatFs`] that lets callers install custom providers (clock,
/// codepage) before mounting.
///
/// Construct via [`FatFs::builder`]. Call [`open`](Self::open) once configured.
/// Without any with_* calls, [`open`](Self::open) behaves identically to
/// [`FatFs::open`].
pub struct FatFsBuilder<DATA: Read + Seek> {
    data: DATA,
    time_provider: &'static dyn crate::time::TimeProvider,
    oem_converter: &'static dyn crate::oem::OemCpConverter,
    /// FAT-cache capacity in sectors, if requested. `None` means no cache.
    #[cfg(feature = "cache")]
    fat_cache_capacity: Option<usize>,
}

impl<DATA: Read + Seek> FatFsBuilder<DATA> {
    /// Start a new builder with default providers.
    pub fn new(data: DATA) -> Self {
        Self {
            data,
            time_provider: &crate::time::DEFAULT_TIME_PROVIDER,
            oem_converter: &crate::oem::DEFAULT_OEM_CONVERTER,
            #[cfg(feature = "cache")]
            fat_cache_capacity: None,
        }
    }

    /// Override the clock used for directory-entry timestamps.
    pub fn with_time_provider(
        mut self,
        provider: &'static dyn crate::time::TimeProvider,
    ) -> Self {
        self.time_provider = provider;
        self
    }

    /// Override the codepage converter used for short (8.3) filenames.
    pub fn with_oem_converter(
        mut self,
        converter: &'static dyn crate::oem::OemCpConverter,
    ) -> Self {
        self.oem_converter = converter;
        self
    }

    /// Install an LRU FAT-sector cache backing read and write operations.
    ///
    /// `capacity_sectors` caps how many FAT sectors the cache holds in
    /// memory at once. Use [`crate::cache::DEFAULT_CACHE_CAPACITY`] (16) as
    /// a sensible starting point. The cache is sync-only — it's silently
    /// not consulted when the filesystem is driven through the async API.
    ///
    /// `capacity_sectors == 0` is treated as "no cache" — the call returns
    /// the builder unchanged rather than installing a degenerate
    /// zero-capacity cache that would refuse every insert.
    ///
    /// Without this call, `FatFs` performs a seek + read on the underlying
    /// data source for every FAT entry access (today's behaviour).
    #[cfg(feature = "cache")]
    pub fn with_fat_cache(mut self, capacity_sectors: usize) -> Self {
        if capacity_sectors == 0 {
            self.fat_cache_capacity = None;
        } else {
            self.fat_cache_capacity = Some(capacity_sectors);
        }
        self
    }

    /// Mount the filesystem with the configured providers.
    pub async fn open(self) -> Result<FatFs<DATA>> {
        #[cfg(feature = "cache")]
        let cap = self.fat_cache_capacity;
        #[cfg(not(feature = "cache"))]
        let fs = FatFs::open_with_providers(self.data, self.time_provider, self.oem_converter).await?;
        #[cfg(feature = "cache")]
        let mut fs = FatFs::open_with_providers(self.data, self.time_provider, self.oem_converter).await?;
        #[cfg(feature = "cache")]
        if let Some(capacity) = cap {
            // Build the cache once we know the FAT layout from the boot sector.
            let (fat_start, fat_size, fat_count, sector_size) = {
                let data = fs.data.lock();
                let sector_size = data.sector_size;
                let (start, size, count) = match &fs.fat {
                    Fat::Fat12(f) => f.cache_layout(),
                    Fat::Fat16(f) => f.cache_layout(),
                    Fat::Fat32(f) => f.cache_layout(),
                };
                (start, size, count, sector_size)
            };
            let cache = crate::cache::FatSectorCache::new(
                fat_start, fat_size, fat_count, sector_size, capacity,
            );
            fs.fat_cache = Some(Mutex::new(cache));
        }
        Ok(fs)
    }
}

/// FAT-resident volume status flags read from `FAT[1]`.
///
/// The FAT spec dedicates two high bits of the cluster-1 entry to volume
/// hygiene: one for "clean shutdown" and one for "no I/O errors during last
/// mount". Both bits are *cleared* when something is wrong. This struct
/// inverts that polarity so a `true` value always means trouble.
///
/// FAT12 has no spare bits in its packed 12-bit entries; the spec doesn't
/// define status flags for FAT12, so a FAT12 mount always reports
/// `dirty: false, io_errors: false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FsStatusFlags {
    /// `true` if the volume was not unmounted cleanly last time.
    pub dirty: bool,
    /// `true` if I/O errors were reported during the last mount.
    pub io_errors: bool,
}

/// FSInfo signature constants
pub(crate) const FSINFO_LEAD_SIG: u32 = 0x41615252; // "RRaA"
pub(crate) const FSINFO_STRUC_SIG: u32 = 0x61417272; // "rrAa"
pub(crate) const FSINFO_TRAIL_SIG: u32 = 0xAA550000;

/// Implementations for Read APIs
impl<DATA> FatFs<DATA>
where
    DATA: Read + Seek,
{
    /// Open a FAT filesystem from a data source with default providers.
    ///
    /// Automatically detects FAT12, FAT16, or FAT32 based on the BPB fields.
    /// Uses [`crate::time::DEFAULT_TIME_PROVIDER`] and
    /// [`crate::oem::DEFAULT_OEM_CONVERTER`]; for custom providers use
    /// [`FatFs::builder`].
    pub async fn open(data: DATA) -> Result<Self> {
        Self::open_with_providers(
            data,
            &crate::time::DEFAULT_TIME_PROVIDER,
            &crate::oem::DEFAULT_OEM_CONVERTER,
        )
        .await
    }

    /// Start a [`FatFsBuilder`] for advanced configuration (custom clock,
    /// codepage, etc.).
    pub fn builder(data: DATA) -> FatFsBuilder<DATA> {
        FatFsBuilder::new(data)
    }

    /// Internal entry point shared by [`open`](Self::open) and
    /// [`FatFsBuilder::open`].
    pub(crate) async fn open_with_providers(
        mut data: DATA,
        time_provider: &'static dyn crate::time::TimeProvider,
        oem_converter: &'static dyn crate::oem::OemCpConverter,
    ) -> Result<Self> {
        // Boot sector is a trust boundary — wrap I/O failures so a truncated
        // or unreadable image surfaces "boot sector" instead of an opaque
        // `Io(...)` and the user knows where to look.
        let bpb = data
            .read_struct::<RawBpb>()
            .await
            .map_err(|source| FatError::IoContext {
                op: "boot sector",
                sector: Some(0),
                source,
            })?;
        let sector_size = bpb.bytes_per_sector.get() as usize;
        let cluster_size = (bpb.sectors_per_cluster as usize) * sector_size;
        let data = SectorCursor::new(data, sector_size, cluster_size);

        // Determine FAT type by checking root_entry_count and sectors_per_fat_16
        // FAT32 has root_entry_count = 0 and sectors_per_fat_16 = 0
        let root_entry_count = u16::from_le_bytes(bpb.root_entry_count);
        let sectors_per_fat_16 = u16::from_le_bytes(bpb.sectors_per_fat_16);

        if root_entry_count == 0 && sectors_per_fat_16 == 0 {
            // FAT32
            Self::open_fat32(data, bpb, time_provider, oem_converter).await
        } else {
            // FAT12 or FAT16
            Self::open_fat12_16(data, bpb, time_provider, oem_converter).await
        }
    }

    /// Open a FAT12/16 filesystem.
    async fn open_fat12_16(
        mut data: SectorCursor<DATA>,
        bpb: RawBpb,
        time_provider: &'static dyn crate::time::TimeProvider,
        oem_converter: &'static dyn crate::oem::OemCpConverter,
    ) -> Result<Self> {
        // Read FAT12/16 extended boot sector
        let bpb_ext16 = data
            .read_struct::<RawBpbExt16>()
            .await
            .map_err(|source| FatError::IoContext {
                op: "boot sector (FAT12/16 extended fields)",
                sector: Some(0),
                source,
            })?;

        // Validate boot signature
        let signature = u16::from_le_bytes(bpb_ext16.signature_word);
        if signature != 0xAA55 {
            return Err(FatError::InvalidBootSignature { found: signature });
        }

        // FAT requires 1 or 2 file allocation tables (BPB_NumFATs). A corrupt
        // count trips a debug_assert deep in the FAT constructors and, in
        // release builds where the assert is stripped, silently corrupts
        // FAT-copy math — reject it here (after the signature check so a
        // non-FAT sector still surfaces InvalidBootSignature first).
        if bpb.fat_count != 1 && bpb.fat_count != 2 {
            return Err(FatError::CorruptFilesystem {
                context: "BPB fat_count must be 1 or 2",
            });
        }

        let sector_size = data.sector_size;
        let cluster_size = data.cluster_size;
        let reserved_sectors = bpb.reserved_sector_count.get() as usize;
        let fat_count = bpb.fat_count as usize;
        let root_entry_count = u16::from_le_bytes(bpb.root_entry_count);
        let sectors_per_fat = u16::from_le_bytes(bpb.sectors_per_fat_16) as usize;

        // Calculate root directory location with checked arithmetic — the
        // BPB fields are untrusted and a corrupt image with absurd values
        // (e.g. sectors_per_fat = 0xFFFF) could otherwise wrap usize on
        // 32-bit targets and seek to garbage.
        let fat_start = reserved_sectors
            .checked_mul(sector_size)
            .ok_or(FatError::CorruptFilesystem {
                context: "reserved_sectors * sector_size",
            })?;
        let fat_total_size = fat_count
            .checked_mul(sectors_per_fat)
            .and_then(|v| v.checked_mul(sector_size))
            .ok_or(FatError::CorruptFilesystem {
                context: "fat_count * sectors_per_fat * sector_size",
            })?;
        let root_dir_start = fat_start
            .checked_add(fat_total_size)
            .ok_or(FatError::CorruptFilesystem {
                context: "fat_start + fat_total_size",
            })?;
        let root_dir_size = (root_entry_count as usize) * 32;
        let root_dir_sectors = root_dir_size.div_ceil(sector_size);

        // Calculate data area start
        let data_start = root_dir_start
            .checked_add(root_dir_sectors * sector_size)
            .ok_or(FatError::CorruptFilesystem {
                context: "data_start arithmetic",
            })?;

        // Calculate total data sectors and cluster count
        let total_sectors = if bpb.total_sectors_16 != [0, 0] {
            u16::from_le_bytes(bpb.total_sectors_16) as u32
        } else {
            u32::from_le_bytes(bpb.total_sectors_32)
        };
        // Saturating subtraction: a corrupt total_sectors smaller than the
        // metadata region size produces 0 data sectors rather than wrapping
        // usize to a huge number.
        let metadata_sectors = reserved_sectors
            .checked_add(fat_count.checked_mul(sectors_per_fat).ok_or(
                FatError::CorruptFilesystem {
                    context: "fat_count * sectors_per_fat",
                },
            )?)
            .and_then(|v| v.checked_add(root_dir_sectors))
            .ok_or(FatError::CorruptFilesystem {
                context: "metadata sector total",
            })?;
        let data_sectors = (total_sectors as usize).saturating_sub(metadata_sectors);
        let count_of_clusters = data_sectors / (bpb.sectors_per_cluster as usize);

        // Determine FAT12 vs FAT16 based on cluster count (per Microsoft spec)
        let (fat, max_cluster) = if count_of_clusters < 4085 {
            // FAT12
            let fat12 = Fat12::new(
                fat_start,
                sectors_per_fat * sector_size,
                fat_count,
                (count_of_clusters + 1) as u16, // +1 because valid clusters are 2..=max
            );
            (Fat::Fat12(fat12), count_of_clusters as u32 + 1)
        } else {
            // FAT16
            let fat16 = Fat16::new(
                fat_start,
                sectors_per_fat * sector_size,
                fat_count,
                (count_of_clusters + 1) as u16,
            );
            (Fat::Fat16(fat16), count_of_clusters as u32 + 1)
        };

        let ext = FatFsExt::Fat12_16(Fat12_16FsExt {
            root_dir_start,
            root_dir_size,
            root_entry_count,
        });

        let info = FatInfo {
            cluster_size,
            data_start,
            max_cluster,
        };

        // Extract volume info from BPB
        let volume_info = VolumeInfo {
            oem_name: bpb.oem_name,
            volume_id: u32::from_le_bytes(bpb_ext16.volume_id),
            volume_label: bpb_ext16.volume_label,
            fs_type_str: bpb_ext16.fs_type,
        };

        Ok(Self {
            data: Mutex::new(data),
            info,
            fat,
            ext,
            volume_info,
            time_provider,
            oem_converter,
            #[cfg(feature = "cache")]
            fat_cache: None,
        })
    }

    /// Open a FAT32 filesystem.
    async fn open_fat32(
        mut data: SectorCursor<DATA>,
        bpb: RawBpb,
        time_provider: &'static dyn crate::time::TimeProvider,
        oem_converter: &'static dyn crate::oem::OemCpConverter,
    ) -> Result<Self> {
        let bpb_ext32 = data
            .read_struct::<RawBpbExt32>()
            .await
            .map_err(|source| FatError::IoContext {
                op: "boot sector (FAT32 extended fields)",
                sector: Some(0),
                source,
            })?;

        // Validate boot signature
        let signature = bpb_ext32.signature_word.get();
        if signature != 0xAA55 {
            return Err(FatError::InvalidBootSignature { found: signature });
        }

        // FAT requires 1 or 2 file allocation tables (BPB_NumFATs) — see the
        // FAT12/16 path. Reject a corrupt count before it reaches Fat32::new's
        // debug_assert (and before it skews FAT-copy math in release).
        if bpb.fat_count != 1 && bpb.fat_count != 2 {
            return Err(FatError::CorruptFilesystem {
                context: "BPB fat_count must be 1 or 2",
            });
        }

        // Read and validate FSInfo
        let fs_info_sec = Sector(bpb_ext32.fs_info_sector.get());
        data.seek_sector(fs_info_sec).await?;
        let fs_info = data
            .read_struct::<RawFsInfo>()
            .await
            .map_err(|source| FatError::IoContext {
                op: "FSInfo",
                sector: Some(fs_info_sec.0 as u64),
                source,
            })?;

        // Validate FSInfo signatures
        let lead_sig = u32::from_le_bytes(fs_info.signature);
        if lead_sig != FSINFO_LEAD_SIG {
            return Err(FatError::InvalidFsInfoSignature {
                field: "FSI_LeadSig",
                expected: FSINFO_LEAD_SIG,
                found: lead_sig,
            });
        }

        let struc_sig = u32::from_le_bytes(fs_info.structure_signature);
        if struc_sig != FSINFO_STRUC_SIG {
            return Err(FatError::InvalidFsInfoSignature {
                field: "FSI_StrucSig",
                expected: FSINFO_STRUC_SIG,
                found: struc_sig,
            });
        }

        let trail_sig = fs_info.trail_signature.get();
        if trail_sig != FSINFO_TRAIL_SIG {
            return Err(FatError::InvalidFsInfoSignature {
                field: "FSI_TrailSig",
                expected: FSINFO_TRAIL_SIG,
                found: trail_sig,
            });
        }

        let ext = FatFsExt::Fat32(Fat32FsExt {
            fs_info_sec,
            root_clus: Cluster(bpb_ext32.root_cluster.get()),
            free_count: Cell::new(fs_info.free_count.get()),
            next_free: Cell::new(Cluster(fs_info.next_free.get())),
        });

        let cluster_size = data.cluster_size;
        let fat_start = Sector(bpb.reserved_sector_count.get()).to_bytes(data.sector_size);
        let fat_size_per_fat =
            Sector(bpb_ext32.sectors_per_fat_32.get()).to_bytes(data.sector_size);
        let fat_size = bpb.fat_count as usize * fat_size_per_fat;

        // Calculate total data sectors and max cluster
        let total_sectors = if bpb.total_sectors_16 != [0, 0] {
            u16::from_le_bytes(bpb.total_sectors_16) as u32
        } else {
            u32::from_le_bytes(bpb.total_sectors_32)
        };
        let reserved_sectors = bpb.reserved_sector_count.get() as u32;
        let fat_sectors = bpb_ext32.sectors_per_fat_32.get() * bpb.fat_count as u32;
        let data_sectors = total_sectors.saturating_sub(reserved_sectors + fat_sectors);
        let max_cluster = (data_sectors / bpb.sectors_per_cluster as u32) + 1; // +1 because clusters start at 2

        let fat = Fat::Fat32(Fat32::new(
            fat_start,
            fat_size_per_fat,
            bpb.fat_count as usize,
            max_cluster,
        ));

        let info = FatInfo {
            cluster_size,
            data_start: fat_start + fat_size,
            max_cluster,
        };

        // Extract volume info from BPB
        let volume_info = VolumeInfo {
            oem_name: bpb.oem_name,
            volume_id: u32::from_le_bytes(bpb_ext32.volume_id),
            volume_label: bpb_ext32.volume_label,
            fs_type_str: bpb_ext32.fs_type,
        };

        Ok(Self {
            data: Mutex::new(data),
            info,
            fat,
            ext,
            volume_info,
            time_provider,
            oem_converter,
            #[cfg(feature = "cache")]
            fat_cache: None,
        })
    }

    /// Borrow the configured clock used for new directory-entry timestamps.
    pub fn time_provider(&self) -> &dyn crate::time::TimeProvider {
        self.time_provider
    }

    /// Borrow the configured OEM codepage converter for short (8.3) names.
    pub fn oem_converter(&self) -> &dyn crate::oem::OemCpConverter {
        self.oem_converter
    }

    /// Borrow the FAT table descriptor.
    ///
    /// Required when constructing a `CachedFat` (with the `cache` feature) via
    /// `CachedFat::new`, which needs the FAT type and
    /// max-cluster bound. Otherwise rarely needed by callers — most FAT
    /// operations go through [`FatFs`] methods directly.
    pub fn fat(&self) -> &Fat {
        &self.fat
    }

    pub fn root_dir(&self) -> FatDir<'_, DATA> {
        match &self.ext {
            FatFsExt::Fat12_16(ext) => FatDir {
                data: self,
                cluster: Cluster(0), // Sentinel for fixed root directory
                fixed_root: Some((ext.root_dir_start, ext.root_dir_size)),
            },
            FatFsExt::Fat32(ext) => FatDir {
                data: self,
                cluster: Cluster(ext.root_clus.0 as usize),
                fixed_root: None,
            },
        }
    }

    /// Get the FAT type of this filesystem
    pub fn fat_type(&self) -> FatType {
        self.fat.fat_type()
    }

    /// Get volume metadata from the boot sector.
    ///
    /// This includes the OEM name, volume serial number, volume label,
    /// and filesystem type string.
    pub fn volume_info(&self) -> &VolumeInfo {
        &self.volume_info
    }

    /// Get the fixed root directory info for FAT12/16 filesystems.
    ///
    /// Returns `Some((start_offset, size))` for FAT12/16, `None` for FAT32.
    #[allow(dead_code)]
    pub(crate) fn fixed_root_dir_info(&self) -> Option<(usize, usize)> {
        self.ext.fixed_root_dir()
    }

    /// Returns true iff `cluster` is the FAT32 root directory cluster.
    ///
    /// Used by directory-creation code to honor the FAT32 spec rule that a
    /// subdirectory's ".." entry must store cluster 0 (not the real root
    /// cluster) when its parent is the FAT32 root.
    #[cfg(feature = "write")]
    pub(crate) fn is_fat32_root_cluster(&self, cluster: u32) -> bool {
        matches!(&self.ext, FatFsExt::Fat32(ext) if ext.root_clus.0 == cluster)
    }

    /// Read the FAT-resident volume status flags from `FAT[1]`.
    ///
    /// `dirty` means the volume was not unmounted cleanly; `io_errors` means
    /// the previous host saw I/O failures. FAT12 has no status bits, so the
    /// returned flags are always `false` for FAT12 — check
    /// [`Self::fat_type`] if that distinction matters to your caller.
    pub async fn read_status_flags(&self) -> Result<FsStatusFlags> {
        let (dirty, io_errors) = self.read_status_flags_routed().await?;
        Ok(FsStatusFlags { dirty, io_errors })
    }

    /// Read the volume label from the root directory entry, if present.
    ///
    /// Two volume labels live on a FAT volume: one in the BPB (boot sector,
    /// always present, available via [`Self::volume_info`]) and an optional
    /// directory entry in the root with the `VOLUME_ID` attribute. Windows
    /// updates the latter when a user renames the volume; the BPB copy can
    /// drift. Use this method to read the authoritative on-disk name.
    ///
    /// Returns `Ok(None)` if no label entry exists.
    pub async fn read_root_label(&self) -> Result<Option<[u8; 11]>> {
        match self.find_root_label_entry().await? {
            Some((_, raw)) => Ok(Some(unsafe { raw.file }.name)),
            None => Ok(None),
        }
    }

    /// Locate the first non-deleted, non-LFN root entry whose attribute set
    /// is exactly `VOLUME_ID` (i.e. a real volume-label entry, not a stray
    /// LFN component which has every bit in `LONG_NAME` set).
    ///
    /// Returns `Ok(Some((byte_pos, raw_entry)))` if found; `Ok(None)` if the
    /// root iterates to its terminator without a label entry.
    pub(crate) async fn find_root_label_entry(
        &self,
    ) -> Result<Option<(usize, crate::raw::RawDirectoryEntry)>> {
        use crate::raw::{DirEntryAttrFlags, RawDirectoryEntry};
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let mut data = self.data.lock();

        let is_label = |attr: u8| {
            let flags = DirEntryAttrFlags::from_bits_retain(attr);
            flags.contains(DirEntryAttrFlags::VOLUME_ID)
                && !flags.contains(DirEntryAttrFlags::DIRECTORY)
                && flags != DirEntryAttrFlags::LONG_NAME
        };

        match &self.ext {
            FatFsExt::Fat12_16(ext) => {
                let end = ext.root_dir_start + ext.root_dir_size;
                let mut pos = ext.root_dir_start;
                while pos + entry_size <= end {
                    data.seek(SeekFrom::Start(pos as u64)).await?;
                    let raw = data.read_struct::<RawDirectoryEntry>().await?;
                    let bytes = unsafe { raw.bytes };
                    if bytes[0] == 0 {
                        return Ok(None);
                    }
                    if bytes[0] != 0xE5 && is_label(unsafe { raw.file }.attributes) {
                        return Ok(Some((pos, raw)));
                    }
                    pos += entry_size;
                }
                Ok(None)
            }
            FatFsExt::Fat32(ext) => {
                let cluster_size = data.cluster_size;
                let mut current = ext.root_clus.0 as usize;
                let chain_limit = self.fat.max_cluster();
                let mut steps: u32 = 0;
                loop {
                    steps = steps.saturating_add(1);
                    if steps > chain_limit {
                        return Err(FatError::ClusterLoop { cluster: current as u32 });
                    }
                    let cluster_start = Cluster(current).to_bytes(self.info.data_start, cluster_size);
                    let mut offset = 0;
                    while offset + entry_size <= cluster_size {
                        let pos = cluster_start + offset;
                        data.seek(SeekFrom::Start(pos as u64)).await?;
                        let raw = data.read_struct::<RawDirectoryEntry>().await?;
                        let bytes = unsafe { raw.bytes };
                        if bytes[0] == 0 {
                            return Ok(None);
                        }
                        if bytes[0] != 0xE5 && is_label(unsafe { raw.file }.attributes) {
                            return Ok(Some((pos, raw)));
                        }
                        offset += entry_size;
                    }
                    // Drop the data lock before calling the routed helper —
                    // it acquires cache+data in canonical order and would
                    // deadlock if we still held data.
                    drop(data);
                    let next_cluster = self.next_cluster_routed(current).await?;
                    data = self.data.lock();
                    match next_cluster {
                        Some(next) => current = next as usize,
                        None => return Ok(None),
                    }
                }
            }
        }
    }

    /// Open a file or directory by path (e.g., "/dir/subdir/file.txt").
    ///
    /// Paths can use forward slashes as separators. Leading slashes are optional.
    /// Empty path components are ignored.
    pub async fn open_path(&self, path: &str) -> Result<FileEntry> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let mut components = path.split('/').filter(|s| !s.is_empty()).peekable();
        if components.peek().is_none() {
            return Err(FatError::InvalidPath);
        }

        let mut current_dir = self.root_dir();
        let mut last_component = None;

        for component in components {
            if let Some(prev) = last_component.take() {
                // Navigate into the previous component as a directory
                current_dir = current_dir.open_dir(prev).await?;
            }
            last_component = Some(component);
        }

        // Find the final entry
        let final_name = last_component.unwrap();
        current_dir.find(final_name).await?.ok_or(FatError::EntryNotFound)
    }

    /// Open a file by path for reading.
    ///
    /// This is a convenience method that combines [`open_path`](Self::open_path)
    /// with opening a file reader.
    pub async fn open_file_path(&self, path: &str) -> Result<FileReader<'_, DATA>> {
        let entry = self.open_path(path).await?;
        FileReader::new(self, &entry)
    }

    /// Open a directory by path.
    ///
    /// This is a convenience method that combines [`open_path`](Self::open_path)
    /// with validating the entry is a directory.
    pub async fn open_dir_path(&self, path: &str) -> Result<FatDir<'_, DATA>> {
        let entry = self.open_path(path).await?;
        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }
        // Subdirectories opened by path are never fixed root
        Ok(FatDir {
            data: self,
            cluster: entry.cluster(),
            fixed_root: None,
        })
    }

    /// Open a directory from a file entry.
    ///
    /// The entry must be a directory.
    pub fn open_dir_entry(&self, entry: &FileEntry) -> Result<FatDir<'_, DATA>> {
        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }
        Ok(FatDir {
            data: self,
            cluster: entry.cluster(),
            fixed_root: None,
        })
    }
}

} // end io_transform!

// ===========================================================================
// Sync-only cache accessors
//
// These expose the installed FAT-sector cache to callers. They reference the
// sync-only `crate::cache` types (`FatSectorCache`, `CachedFat`) and call
// their synchronous I/O methods, so they are emitted only in the sync slice.
// Under the async API the cache is bypassed (a build with both `async` and
// `cache` keeps the field but offers no async cache accessors), which is what
// lets `--features async,cache` (and `--all-features`) compile.
// ===========================================================================

#[cfg(feature = "cache")]
sync_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + Seek,
    {
        /// Borrow the optional FAT-sector cache configured via
        /// [`FatFsBuilder::with_fat_cache`].
        ///
        /// Returns `None` if no cache was installed. Pair with [`Self::fat`]
        /// and [`crate::cache::CachedFat::new`] to perform cached FAT
        /// operations, or use the higher-level [`Self::with_cached_fat`]
        /// helper which holds the cache and disk locks for you.
        pub fn fat_cache(&self) -> Option<&Mutex<crate::cache::FatSectorCache>> {
            self.fat_cache.as_ref()
        }

        /// Run a closure with a [`crate::cache::CachedFat`] view backed by this
        /// filesystem's installed FAT cache and underlying disk handle.
        ///
        /// Returns `None` if no cache was installed via
        /// [`FatFsBuilder::with_fat_cache`]. Otherwise locks the cache mutex
        /// and the data mutex for the duration of the closure and returns
        /// `Some(value)` where `value` is the closure's return.
        ///
        /// As of phase C5, `FatFs`'s built-in methods consult the cache
        /// automatically; this helper remains useful for bulk FAT walks
        /// (free-cluster scans, multi-chain traversal) where holding the
        /// cache+disk locks across many entries is cheaper than re-acquiring
        /// them per call.
        ///
        /// # Example
        ///
        /// ```rust,no_run
        /// # #[cfg(all(feature = "cache", feature = "std"))]
        /// # {
        /// use std::fs::OpenOptions;
        /// use hadris_fat::FatFs;
        ///
        /// let disk = OpenOptions::new().read(true).write(true).open("disk.img").unwrap();
        /// let fs = FatFs::builder(disk).with_fat_cache(16).open().unwrap();
        ///
        /// // Walk the cluster chain of the file at first_cluster=42, using the cache.
        /// let chain = fs
        ///     .with_cached_fat(|cached, disk| cached.read_chain(disk, 42))
        ///     .expect("cache installed")
        ///     .expect("read_chain ok");
        /// # }
        /// ```
        pub fn with_cached_fat<R>(
            &self,
            f: impl FnOnce(&mut crate::cache::CachedFat<'_>, &mut SectorCursor<DATA>) -> R,
        ) -> Option<R> {
            let cache_mutex = self.fat_cache.as_ref()?;
            let mut cache = cache_mutex.lock();
            let mut data = self.data.lock();
            let mut cached = crate::cache::CachedFat::new(&mut *cache, &self.fat);
            Some(f(&mut cached, &mut *data))
        }

        /// Run a closure with both the [`crate::cache::FatSectorCache`] and
        /// underlying disk locked for direct, FAT-type-specific access.
        ///
        /// Lower-level than [`Self::with_cached_fat`] — gives the closure
        /// `&mut FatSectorCache` so it can call the per-type entry-point
        /// methods ([`crate::cache::FatSectorCache::read_fat32_entry`],
        /// [`crate::cache::FatSectorCache::write_fat32_entry`], etc.). Most
        /// callers want [`Self::with_cached_fat`] instead, which wraps the
        /// cache in a `CachedFat` and hides the FAT-type dispatch.
        ///
        /// Note: do NOT call [`Self::fat_cache`]`.lock()` inside this closure —
        /// the cache mutex is already locked, so a second lock attempt will
        /// deadlock (this is a `spin::Mutex`, not a re-entrant lock).
        ///
        /// Returns `None` if no cache was installed.
        pub fn with_fat_cache_locked<R>(
            &self,
            f: impl FnOnce(&mut crate::cache::FatSectorCache, &mut SectorCursor<DATA>) -> R,
        ) -> Option<R> {
            let cache_mutex = self.fat_cache.as_ref()?;
            let mut cache = cache_mutex.lock();
            let mut data = self.data.lock();
            Some(f(&mut *cache, &mut *data))
        }
    }
}

// ===========================================================================
// Sync-only cache routing (Phase C5)
//
// `cache.rs` is sync-only (its methods don't await), so the routed
// FAT-table helpers below are emitted only in the sync slice. The async
// slice gets a thin pass-through impl from `async_only!` further down so
// callers in `io_transform!{}` can always write `self.next_cluster_routed(...).await?`.
//
// Lock ordering invariant: cache mutex first, then data mutex — matches
// `with_cached_fat`. Callers MUST NOT hold the data mutex when entering
// these helpers (spin::Mutex is not reentrant).
// ===========================================================================

#[cfg(feature = "cache")]
sync_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + Seek,
    {
        /// Read the next cluster of `cluster`, routing through the FAT-sector
        /// cache if one is installed.
        ///
        /// Caller must NOT hold `self.data` — this method acquires both
        /// locks (cache then data) in canonical order. Returns the same
        /// `Result<Option<u32>>` as [`Fat::next_cluster`].
        pub(crate) fn next_cluster_routed(&self, cluster: usize) -> Result<Option<u32>> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(cache) = cache_guard.as_mut() {
                let mut cached = crate::cache::CachedFat::new(cache, &self.fat);
                cached.next_cluster(data.deref_mut(), cluster)
            } else {
                self.fat.next_cluster(data.deref_mut(), cluster)
            }
        }

        /// Read `FAT[1]` status flags through the cache when installed.
        pub(crate) fn read_status_flags_routed(&self) -> Result<(bool, bool)> {
            use core::ops::DerefMut;
            // FAT12 has no status bits regardless of cache installation;
            // skip the cache lock entirely.
            if matches!(self.fat, Fat::Fat12(_)) {
                return Ok((false, false));
            }
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            match (&self.fat, cache_guard.as_deref_mut()) {
                (Fat::Fat16(_), Some(cache)) => {
                    let val = cache.read_fat16_entry(data.deref_mut(), 1)?;
                    Ok((val & 0x8000 == 0, val & 0x4000 == 0))
                }
                (Fat::Fat32(_), Some(cache)) => {
                    let val = cache.read_fat32_entry(data.deref_mut(), 1)?;
                    Ok((val & 0x0800_0000 == 0, val & 0x0400_0000 == 0))
                }
                _ => self.fat.read_status_flags(data.deref_mut()),
            }
        }
    }
}

#[cfg(feature = "cache")]
async_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + Seek,
    {
        /// Async pass-through: cache routing is sync-only (deferred to phase C5b).
        pub(crate) async fn next_cluster_routed(&self, cluster: usize) -> Result<Option<u32>> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.next_cluster(data.deref_mut(), cluster).await
        }

        /// Async pass-through.
        pub(crate) async fn read_status_flags_routed(&self) -> Result<(bool, bool)> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.read_status_flags(data.deref_mut()).await
        }
    }
}

// When the `cache` feature is off, the routed helpers are simple
// pass-throughs that drop the cache layer entirely. Defining them here
// keeps `io_transform!{}` call sites uniform regardless of feature flags.
#[cfg(not(feature = "cache"))]
io_transform! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + Seek,
    {
        pub(crate) async fn next_cluster_routed(&self, cluster: usize) -> Result<Option<u32>> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.next_cluster(data.deref_mut(), cluster).await
        }

        pub(crate) async fn read_status_flags_routed(&self) -> Result<(bool, bool)> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.read_status_flags(data.deref_mut()).await
        }
    }
}

// ===========================================================================
// Write routing (Phase C5)
// ===========================================================================
//
// When the cache is installed, FAT-table mutations must go through the cache
// to keep cached read state coherent with on-disk writes (see
// `writes_then_reads_through_cache_are_consistent` in
// `tests/cache_integration.rs`).
//
// `cache.rs` exposes `write_fat{12,16,32}_entry` for individual entry writes;
// the higher-level operations (allocate / free / truncate / mark_bad) are
// reimplemented here against those primitives. When no cache is installed we
// fall through to the existing `Fat::*` helpers, preserving today's
// performance characteristics.
//
// Async builds receive thin pass-throughs: the cache feature requires `sync`
// (Cargo.toml), so a build that lacks `sync` cannot reach these methods.

#[cfg(all(feature = "cache", feature = "write"))]
sync_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + super::io::Write + Seek,
    {
        /// Write a single FAT entry through the cache when installed.
        pub(crate) fn write_clus_routed(&self, cluster: usize, value: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(ref mut cache) = cache_guard {
                match self.fat.fat_type() {
                    FatType::Fat12 => cache.write_fat12_entry(data.deref_mut(), cluster, value as u16),
                    FatType::Fat16 => cache.write_fat16_entry(data.deref_mut(), cluster, value as u16),
                    FatType::Fat32 => cache.write_fat32_entry(data.deref_mut(), cluster, value),
                }
            } else {
                match &self.fat {
                    Fat::Fat12(f) => f.write_clus(data.deref_mut(), cluster, value as u16),
                    Fat::Fat16(f) => f.write_clus(data.deref_mut(), cluster, value as u16),
                    Fat::Fat32(f) => f.write_clus(data.deref_mut(), cluster, value),
                }
            }
        }

        /// Allocate a single cluster, returning its number. Routes through
        /// the cache when installed; otherwise falls through to `Fat::*`.
        pub(crate) fn allocate_cluster_routed(&self, hint: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(ref mut cache) = cache_guard {
                allocate_cluster_via_cache(cache, &self.fat, data.deref_mut(), hint)
            } else {
                match &self.fat {
                    Fat::Fat12(f) => f.allocate_cluster(data.deref_mut(), hint as u16).map(|c| c as u32),
                    Fat::Fat16(f) => f.allocate_cluster(data.deref_mut(), hint as u16).map(|c| c as u32),
                    Fat::Fat32(f) => f.allocate_cluster(data.deref_mut(), hint),
                }
            }
        }

        /// Free a cluster chain starting at `start`, returning the count of
        /// freed clusters. Routes through the cache when installed.
        pub(crate) fn free_chain_routed(&self, start: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(ref mut cache) = cache_guard {
                free_chain_via_cache(cache, &self.fat, data.deref_mut(), start)
            } else {
                self.fat.free_chain(data.deref_mut(), start as usize)
            }
        }

        /// Truncate a chain after the specified cluster (the cluster
        /// becomes the new EOC; everything after is freed).
        pub(crate) fn truncate_chain_routed(&self, cluster: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(ref mut cache) = cache_guard {
                truncate_chain_via_cache(cache, &self.fat, data.deref_mut(), cluster)
            } else {
                self.fat.truncate_chain(data.deref_mut(), cluster as usize)
            }
        }

        /// Mark a cluster as bad in the FAT.
        #[allow(dead_code)] // Surfaced for future fsck-style tooling; nothing in
        // this crate calls `Fat::mark_bad` today, but the routed variant is
        // kept symmetric with the rest of the cache surface so a follow-up
        // does not have to re-derive lock ordering.
        pub(crate) fn mark_bad_routed(&self, cluster: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut cache_guard = self.fat_cache.as_ref().map(|m| m.lock());
            let mut data = self.data.lock();
            if let Some(ref mut cache) = cache_guard {
                let bad = match self.fat.fat_type() {
                    FatType::Fat12 => return cache.write_fat12_entry(data.deref_mut(), cluster as usize, 0x0FF7),
                    FatType::Fat16 => return cache.write_fat16_entry(data.deref_mut(), cluster as usize, 0xFFF7),
                    FatType::Fat32 => 0x0FFF_FFF7u32,
                };
                cache.write_fat32_entry(data.deref_mut(), cluster as usize, bad)
            } else {
                self.fat.mark_bad(data.deref_mut(), cluster as usize)
            }
        }
    }
}

#[cfg(all(feature = "cache", feature = "write"))]
async_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + super::io::Write + Seek,
    {
        /// Async pass-through; cache routing is sync-only (deferred to phase C5b).
        pub(crate) async fn write_clus_routed(&self, cluster: usize, value: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            match &self.fat {
                Fat::Fat12(f) => f.write_clus(data.deref_mut(), cluster, value as u16).await,
                Fat::Fat16(f) => f.write_clus(data.deref_mut(), cluster, value as u16).await,
                Fat::Fat32(f) => f.write_clus(data.deref_mut(), cluster, value).await,
            }
        }

        pub(crate) async fn allocate_cluster_routed(&self, hint: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            match &self.fat {
                Fat::Fat12(f) => f.allocate_cluster(data.deref_mut(), hint as u16).await.map(|c| c as u32),
                Fat::Fat16(f) => f.allocate_cluster(data.deref_mut(), hint as u16).await.map(|c| c as u32),
                Fat::Fat32(f) => f.allocate_cluster(data.deref_mut(), hint).await,
            }
        }

        pub(crate) async fn free_chain_routed(&self, start: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.free_chain(data.deref_mut(), start as usize).await
        }

        pub(crate) async fn truncate_chain_routed(&self, cluster: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.truncate_chain(data.deref_mut(), cluster as usize).await
        }

        #[allow(dead_code)]
        pub(crate) async fn mark_bad_routed(&self, cluster: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.mark_bad(data.deref_mut(), cluster as usize).await
        }
    }
}

// When `cache` is off, callers in `io_transform!{}` still write
// `self.write_clus_routed(...).await?`. Provide a uniform pass-through.
#[cfg(all(not(feature = "cache"), feature = "write"))]
io_transform! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + super::io::Write + Seek,
    {
        pub(crate) async fn write_clus_routed(&self, cluster: usize, value: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            match &self.fat {
                Fat::Fat12(f) => f.write_clus(data.deref_mut(), cluster, value as u16).await,
                Fat::Fat16(f) => f.write_clus(data.deref_mut(), cluster, value as u16).await,
                Fat::Fat32(f) => f.write_clus(data.deref_mut(), cluster, value).await,
            }
        }

        pub(crate) async fn allocate_cluster_routed(&self, hint: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            match &self.fat {
                Fat::Fat12(f) => f.allocate_cluster(data.deref_mut(), hint as u16).await.map(|c| c as u32),
                Fat::Fat16(f) => f.allocate_cluster(data.deref_mut(), hint as u16).await.map(|c| c as u32),
                Fat::Fat32(f) => f.allocate_cluster(data.deref_mut(), hint).await,
            }
        }

        pub(crate) async fn free_chain_routed(&self, start: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.free_chain(data.deref_mut(), start as usize).await
        }

        pub(crate) async fn truncate_chain_routed(&self, cluster: u32) -> Result<u32> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.truncate_chain(data.deref_mut(), cluster as usize).await
        }

        #[allow(dead_code)]
        pub(crate) async fn mark_bad_routed(&self, cluster: u32) -> Result<()> {
            use core::ops::DerefMut;
            let mut data = self.data.lock();
            self.fat.mark_bad(data.deref_mut(), cluster as usize).await
        }
    }
}

// Free helpers used by the sync cache path. Kept here (not in cache.rs) so
// the cache module's API stays untouched per the C5 plan. Wrapped in
// `sync_only!` so they exist only in the sync slice — they invoke the
// synchronous `FatSectorCache` methods and so cannot compile in the async
// slice (where `super::io` is the async trait set). This is what lets
// `async + cache` build with the cache simply bypassed.
#[cfg(all(feature = "cache", feature = "write"))]
sync_only! {

fn allocate_cluster_via_cache<T>(
    cache: &mut crate::cache::FatSectorCache,
    fat: &Fat,
    data: &mut T,
    hint: u32,
) -> Result<u32>
where
    T: super::io::Read + super::io::Write + super::io::Seek,
{
    const FIRST: u32 = 2;
    let max_cluster = fat.max_cluster();
    let start = if hint >= FIRST && hint <= max_cluster {
        hint
    } else {
        FIRST
    };

    let scan = |cache: &mut crate::cache::FatSectorCache,
                data: &mut T,
                fat: &Fat,
                lo: u32,
                hi: u32|
     -> Result<Option<u32>> {
        for c in lo..=hi {
            let free = match fat.fat_type() {
                FatType::Fat12 => (cache.read_fat12_entry(data, c as usize)? & 0x0FFF) == 0,
                FatType::Fat16 => cache.read_fat16_entry(data, c as usize)? == 0,
                FatType::Fat32 => (cache.read_fat32_entry(data, c as usize)? & 0x0FFF_FFFF) == 0,
            };
            if free {
                return Ok(Some(c));
            }
        }
        Ok(None)
    };

    let claim =
        |cache: &mut crate::cache::FatSectorCache, data: &mut T, fat: &Fat, c: u32| -> Result<()> {
            match fat.fat_type() {
                FatType::Fat12 => cache.write_fat12_entry(data, c as usize, 0x0FF8),
                FatType::Fat16 => cache.write_fat16_entry(data, c as usize, 0xFFF8),
                FatType::Fat32 => cache.write_fat32_entry(data, c as usize, 0x0FFF_FFF8),
            }
        };

    if let Some(c) = scan(cache, data, fat, start, max_cluster)? {
        claim(cache, data, fat, c)?;
        return Ok(c);
    }
    if start > FIRST
        && let Some(c) = scan(cache, data, fat, FIRST, start - 1)?
    {
        claim(cache, data, fat, c)?;
        return Ok(c);
    }
    Err(FatError::NoFreeSpace)
}

#[cfg(all(feature = "cache", feature = "write"))]
fn free_chain_via_cache<T>(
    cache: &mut crate::cache::FatSectorCache,
    fat: &Fat,
    data: &mut T,
    start: u32,
) -> Result<u32>
where
    T: super::io::Read + super::io::Write + super::io::Seek,
{
    const FIRST: u32 = 2;
    let max_cluster = fat.max_cluster();
    let mut count = 0u32;
    let mut current = start;
    loop {
        if current < FIRST || current > max_cluster {
            break;
        }
        let next = read_fat_entry_via_cache(cache, fat, data, current as usize)?;
        write_fat_entry_via_cache(cache, fat, data, current as usize, 0)?;
        count += 1;
        if is_eoc(fat.fat_type(), next) || is_bad(fat.fat_type(), next) || next == 0 {
            break;
        }
        current = next;
    }
    Ok(count)
}

#[cfg(all(feature = "cache", feature = "write"))]
fn truncate_chain_via_cache<T>(
    cache: &mut crate::cache::FatSectorCache,
    fat: &Fat,
    data: &mut T,
    cluster: u32,
) -> Result<u32>
where
    T: super::io::Read + super::io::Write + super::io::Seek,
{
    const FIRST: u32 = 2;
    let max_cluster = fat.max_cluster();
    if cluster < FIRST || cluster > max_cluster {
        return Ok(0);
    }
    let next = read_fat_entry_via_cache(cache, fat, data, cluster as usize)?;
    let eoc = match fat.fat_type() {
        FatType::Fat12 => 0x0FF8,
        FatType::Fat16 => 0xFFF8,
        FatType::Fat32 => 0x0FFF_FFF8,
    };
    write_fat_entry_via_cache(cache, fat, data, cluster as usize, eoc)?;
    if !is_eoc(fat.fat_type(), next) && next >= FIRST && next <= max_cluster {
        free_chain_via_cache(cache, fat, data, next)
    } else {
        Ok(0)
    }
}

#[cfg(all(feature = "cache", feature = "write"))]
fn read_fat_entry_via_cache<T>(
    cache: &mut crate::cache::FatSectorCache,
    fat: &Fat,
    data: &mut T,
    cluster: usize,
) -> Result<u32>
where
    T: super::io::Read + super::io::Seek,
{
    Ok(match fat.fat_type() {
        FatType::Fat12 => (cache.read_fat12_entry(data, cluster)? & 0x0FFF) as u32,
        FatType::Fat16 => cache.read_fat16_entry(data, cluster)? as u32,
        FatType::Fat32 => cache.read_fat32_entry(data, cluster)? & 0x0FFF_FFFF,
    })
}

#[cfg(all(feature = "cache", feature = "write"))]
fn write_fat_entry_via_cache<T>(
    cache: &mut crate::cache::FatSectorCache,
    fat: &Fat,
    data: &mut T,
    cluster: usize,
    value: u32,
) -> Result<()>
where
    T: super::io::Read + super::io::Write + super::io::Seek,
{
    match fat.fat_type() {
        FatType::Fat12 => cache.write_fat12_entry(data, cluster, value as u16),
        FatType::Fat16 => cache.write_fat16_entry(data, cluster, value as u16),
        FatType::Fat32 => cache.write_fat32_entry(data, cluster, value),
    }
}

#[cfg(all(feature = "cache", feature = "write"))]
fn is_eoc(ty: FatType, value: u32) -> bool {
    match ty {
        FatType::Fat12 => value >= 0x0FF8,
        FatType::Fat16 => value >= 0xFFF8,
        FatType::Fat32 => value >= 0x0FFF_FFF8,
    }
}

#[cfg(all(feature = "cache", feature = "write"))]
fn is_bad(ty: FatType, value: u32) -> bool {
    match ty {
        FatType::Fat12 => value == 0x0FF7,
        FatType::Fat16 => value == 0xFFF7,
        FatType::Fat32 => value == 0x0FFF_FFF7,
    }
}

} // end sync_only! (free cache helpers)

// ===========================================================================
// Flush
// ===========================================================================

// Flush is only available with `cache` + `write`: nothing to flush without a
// cache, and a writable backing store is required to mirror dirty sectors to
// every FAT copy. Sync-only because `FatSectorCache::flush` uses synchronous
// I/O traits.
#[cfg(all(feature = "cache", feature = "write"))]
sync_only! {
    impl<DATA> FatFs<DATA>
    where
        DATA: Read + super::io::Write + Seek,
    {
        /// Flush all dirty FAT cache sectors back to every FAT copy on disk.
        ///
        /// No-op when no cache was installed. Without an explicit `flush()`,
        /// dirty sectors are written through to disk on LRU eviction (see
        /// `FatSectorCache::evict_lru_flush`) or are still in memory when the
        /// [`FatFs`] is dropped. Call this before tearing down the
        /// filesystem to guarantee the on-disk FAT is consistent.
        pub fn flush(&self) -> Result<()> {
            use core::ops::DerefMut;
            if let Some(cache_mutex) = &self.fat_cache {
                let mut cache = cache_mutex.lock();
                let mut data = self.data.lock();
                cache.flush(data.deref_mut())?;
            }
            Ok(())
        }
    }
}

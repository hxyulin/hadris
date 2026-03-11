io_transform! {

use core::{cell::Cell, fmt};

use spin::Mutex;

use hadris_common::types::endian::Endian;

use crate::error::{FatError, Result};
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};
use super::dir::{FatDir, FileEntry};
use super::fat_table::{Fat, Fat12, Fat16, Fat32, FatType};
use super::io::{Cluster, Read, ReadExt, Sector, SectorCursor, SectorLike, Seek};
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
}

impl<DATA: Seek> fmt::Debug for FatFs<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("info", &self.info)
            .field("ext", &self.ext)
            .finish_non_exhaustive()
    }
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
    /// Open a FAT filesystem from a data source.
    ///
    /// Automatically detects FAT12, FAT16, or FAT32 based on the BPB fields.
    pub async fn open(mut data: DATA) -> Result<Self> {
        let bpb = data.read_struct::<RawBpb>().await?;
        let sector_size = bpb.bytes_per_sector.get() as usize;
        let cluster_size = (bpb.sectors_per_cluster as usize) * sector_size;
        let data = SectorCursor::new(data, sector_size, cluster_size);

        // Determine FAT type by checking root_entry_count and sectors_per_fat_16
        // FAT32 has root_entry_count = 0 and sectors_per_fat_16 = 0
        let root_entry_count = u16::from_le_bytes(bpb.root_entry_count);
        let sectors_per_fat_16 = u16::from_le_bytes(bpb.sectors_per_fat_16);

        if root_entry_count == 0 && sectors_per_fat_16 == 0 {
            // FAT32
            Self::open_fat32(data, bpb).await
        } else {
            // FAT12 or FAT16
            Self::open_fat12_16(data, bpb).await
        }
    }

    /// Open a FAT12/16 filesystem.
    async fn open_fat12_16(mut data: SectorCursor<DATA>, bpb: RawBpb) -> Result<Self> {
        // Read FAT12/16 extended boot sector
        let bpb_ext16 = data.read_struct::<RawBpbExt16>().await?;

        // Validate boot signature
        let signature = u16::from_le_bytes(bpb_ext16.signature_word);
        if signature != 0xAA55 {
            return Err(FatError::InvalidBootSignature { found: signature });
        }

        let sector_size = data.sector_size;
        let cluster_size = data.cluster_size;
        let reserved_sectors = bpb.reserved_sector_count.get() as usize;
        let fat_count = bpb.fat_count as usize;
        let root_entry_count = u16::from_le_bytes(bpb.root_entry_count);
        let sectors_per_fat = u16::from_le_bytes(bpb.sectors_per_fat_16) as usize;

        // Calculate root directory location
        let fat_start = reserved_sectors * sector_size;
        let fat_total_size = fat_count * sectors_per_fat * sector_size;
        let root_dir_start = fat_start + fat_total_size;
        let root_dir_size = (root_entry_count as usize) * 32;
        let root_dir_sectors = root_dir_size.div_ceil(sector_size);

        // Calculate data area start
        let data_start = root_dir_start + root_dir_sectors * sector_size;

        // Calculate total data sectors and cluster count
        let total_sectors = if bpb.total_sectors_16 != [0, 0] {
            u16::from_le_bytes(bpb.total_sectors_16) as u32
        } else {
            u32::from_le_bytes(bpb.total_sectors_32)
        };
        let data_sectors = total_sectors as usize
            - reserved_sectors
            - fat_count * sectors_per_fat
            - root_dir_sectors;
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
        })
    }

    /// Open a FAT32 filesystem.
    async fn open_fat32(mut data: SectorCursor<DATA>, bpb: RawBpb) -> Result<Self> {
        let bpb_ext32 = data.read_struct::<RawBpbExt32>().await?;

        // Validate boot signature
        let signature = bpb_ext32.signature_word.get();
        if signature != 0xAA55 {
            return Err(FatError::InvalidBootSignature { found: signature });
        }

        // Read and validate FSInfo
        let fs_info_sec = Sector(bpb_ext32.fs_info_sector.get());
        data.seek_sector(fs_info_sec).await?;
        let fs_info = data.read_struct::<RawFsInfo>().await?;

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
        })
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
    pub(crate) fn fixed_root_dir_info(&self) -> Option<(usize, usize)> {
        self.ext.fixed_root_dir()
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

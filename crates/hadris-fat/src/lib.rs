#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod error;
pub mod file;
pub mod io;
pub mod read;
pub mod write;

#[cfg(feature = "cache")]
pub mod cache;

#[cfg(feature = "tool")]
pub mod tool;

#[cfg(feature = "exfat")]
pub mod exfat;

#[cfg(feature = "write")]
pub mod format;

pub use error::{FatError, Result};
pub use io::Cluster;
// FatType is already public since it's defined with `pub enum`
pub use read::{FatFsReadExt, FileReader};
#[cfg(feature = "write")]
pub use write::{FatDateTime, FatFsWriteExt, FileWriter};
#[cfg(feature = "write")]
pub use format::{FatVolumeFormatter, FormatOptions, FormatParams};
#[cfg(feature = "cache")]
pub use cache::{CachedFat, CacheStats, FatSectorCache, DEFAULT_CACHE_CAPACITY};
#[cfg(feature = "tool")]
pub use tool::{
    analysis::{ClusterState, FatAnalysisExt, FatStatistics, FileFragmentInfo, FragmentationReport},
    verify::{FatVerifyExt, VerificationIssue, VerificationReport},
};

use core::{cell::Cell, fmt, mem::size_of, ops::DerefMut};
use io::{ClusterLike, Read, ReadExt, Sector, SectorCursor, SectorLike, Seek, SeekFrom};
#[cfg(feature = "write")]
use io::Write;
use spin::Mutex;

use hadris_common::types::{
    endian::{Endian, LittleEndian},
    number::{U16, U32},
};

use crate::file::ShortFileName;
#[cfg(feature = "lfn")]
use crate::file::{LfnBuilder, LongFileName};

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
struct Fat12_16FsExt {
    /// Root directory start byte offset
    root_dir_start: usize,
    /// Root directory size in bytes
    root_dir_size: usize,
    /// Number of root directory entries
    #[allow(dead_code)]
    root_entry_count: u16,
}

#[derive(Debug)]
enum FatFsExt {
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
struct Fat32FsExt {
    /// Sector number of the FSInfo structure
    fs_info_sec: Sector<u16>,
    /// Root directory cluster
    root_clus: Cluster<u32>,
    /// Number of free clusters (from FSInfo, may be stale)
    free_count: Cell<u32>,
    /// Hint for next free cluster (from FSInfo)
    next_free: Cell<Cluster<u32>>,
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
    ext: FatFsExt,
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
const FSINFO_LEAD_SIG: u32 = 0x41615252;    // "RRaA"
const FSINFO_STRUC_SIG: u32 = 0x61417272;   // "rrAa"
const FSINFO_TRAIL_SIG: u32 = 0xAA550000;

/// Implementations for Read APIs
impl<DATA> FatFs<DATA>
where
    DATA: Read + Seek,
{
    /// Open a FAT filesystem from a data source.
    ///
    /// Automatically detects FAT12, FAT16, or FAT32 based on the BPB fields.
    pub fn open(mut data: DATA) -> Result<Self> {
        let bpb = data.read_struct::<RawBpb>()?;
        let sector_size = bpb.bytes_per_sector.get() as usize;
        let cluster_size = (bpb.sectors_per_cluster as usize) * sector_size;
        let data = SectorCursor::new(data, sector_size, cluster_size);

        // Determine FAT type by checking root_entry_count and sectors_per_fat_16
        // FAT32 has root_entry_count = 0 and sectors_per_fat_16 = 0
        let root_entry_count = u16::from_le_bytes(bpb.root_entry_count);
        let sectors_per_fat_16 = u16::from_le_bytes(bpb.sectors_per_fat_16);

        if root_entry_count == 0 && sectors_per_fat_16 == 0 {
            // FAT32
            Self::open_fat32(data, bpb)
        } else {
            // FAT12 or FAT16
            Self::open_fat12_16(data, bpb)
        }
    }

    /// Open a FAT12/16 filesystem.
    fn open_fat12_16(mut data: SectorCursor<DATA>, bpb: RawBpb) -> Result<Self> {
        // Read FAT12/16 extended boot sector
        let bpb_ext16 = data.read_struct::<RawBpbExt16>()?;

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
        let root_dir_sectors = (root_dir_size + sector_size - 1) / sector_size;

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
    fn open_fat32(mut data: SectorCursor<DATA>, bpb: RawBpb) -> Result<Self> {
        let bpb_ext32 = data.read_struct::<RawBpbExt32>()?;

        // Validate boot signature
        let signature = bpb_ext32.signature_word.get();
        if signature != 0xAA55 {
            return Err(FatError::InvalidBootSignature { found: signature });
        }

        // Read and validate FSInfo
        let fs_info_sec = Sector(bpb_ext32.fs_info_sector.get());
        data.seek_sector(fs_info_sec)?;
        let fs_info = data.read_struct::<RawFsInfo>()?;

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
        let fat_size_per_fat = Sector(bpb_ext32.sectors_per_fat_32.get()).to_bytes(data.sector_size);
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

        let fat = Fat::Fat32(Fat32::new(fat_start, fat_size_per_fat, bpb.fat_count as usize, max_cluster));

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
    #[cfg(feature = "alloc")]
    pub fn open_path(&self, path: &str) -> Result<FileEntry> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let components: alloc::vec::Vec<&str> =
            path.split('/').filter(|s| !s.is_empty()).collect();
        if components.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let mut current_dir = self.root_dir();

        // Navigate to parent directories
        for component in &components[..components.len() - 1] {
            current_dir = current_dir.open_dir(component)?;
        }

        // Find the final entry
        let final_name = components.last().unwrap();
        current_dir.find(final_name)?.ok_or(FatError::EntryNotFound)
    }

    /// Open a file by path for reading.
    ///
    /// This is a convenience method that combines [`open_path`](Self::open_path)
    /// with opening a file reader.
    #[cfg(feature = "alloc")]
    pub fn open_file_path(&self, path: &str) -> Result<FileReader<'_, DATA>> {
        let entry = self.open_path(path)?;
        FileReader::new(self, &entry)
    }

    /// Open a directory by path.
    ///
    /// This is a convenience method that combines [`open_path`](Self::open_path)
    /// with validating the entry is a directory.
    #[cfg(feature = "alloc")]
    pub fn open_dir_path(&self, path: &str) -> Result<FatDir<'_, DATA>> {
        let entry = self.open_path(path)?;
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

pub struct FatDir<'a, DATA: Read + Seek> {
    pub(crate) data: &'a FatFs<DATA>,
    /// Cluster for subdirectories, or 0 (sentinel) for FAT12/16 fixed root
    pub(crate) cluster: Cluster,
    /// For FAT12/16 root: (start_byte, size_bytes), None for cluster-based dirs
    pub(crate) fixed_root: Option<(usize, usize)>,
}

impl<'a, DATA: Read + Seek> FatDir<'a, DATA> {
    #[cfg(feature = "lfn")]
    pub fn entries(&self) -> FatDirIter<'a, DATA> {
        FatDirIter {
            data: self.data,
            cluster: self.cluster,
            offset: 0,
            fixed_root_remaining: self.fixed_root.map(|(_, size)| size),
            fixed_root_start: self.fixed_root.map(|(start, _)| start),
            lfn_builder: LfnBuilder::new(),
            #[cfg(feature = "alloc")]
            cluster_buffer: None,
            #[cfg(feature = "alloc")]
            buffer_valid: false,
        }
    }

    #[cfg(not(feature = "lfn"))]
    pub fn entries(&self) -> FatDirIter<'a, DATA> {
        FatDirIter {
            data: self.data,
            cluster: self.cluster,
            offset: 0,
            fixed_root_remaining: self.fixed_root.map(|(_, size)| size),
            fixed_root_start: self.fixed_root.map(|(start, _)| start),
            #[cfg(feature = "alloc")]
            cluster_buffer: None,
            #[cfg(feature = "alloc")]
            buffer_valid: false,
        }
    }

    /// Open a subdirectory from a file entry.
    ///
    /// The entry must be a directory.
    pub fn open_entry(&self, entry: &FileEntry) -> Result<FatDir<'a, DATA>> {
        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }
        Ok(FatDir {
            data: self.data,
            cluster: entry.cluster(),
            fixed_root: None, // Subdirectories are never fixed root
        })
    }

    /// Find an entry by name.
    ///
    /// When the `lfn` feature is enabled, this performs a case-sensitive match
    /// against long file names first, then falls back to case-insensitive short
    /// name matching.
    ///
    /// Without the `lfn` feature, only case-insensitive short name matching is used.
    pub fn find(&self, name: &str) -> Result<Option<FileEntry>> {
        for entry in self.entries() {
            let DirectoryEntry::Entry(file_entry) = entry?;

            // Check LFN match (case-sensitive)
            #[cfg(feature = "lfn")]
            if let Some(lfn) = file_entry.long_name() {
                if lfn.as_str() == name {
                    return Ok(Some(file_entry));
                }
            }
            // Check short name match (case-insensitive, handles 8.3 padding)
            if file_entry.short_name().matches(name) {
                return Ok(Some(file_entry));
            }
        }
        Ok(None)
    }

    /// Open a subdirectory by name.
    ///
    /// Returns an error if the entry is not found or is not a directory.
    pub fn open_dir(&self, name: &str) -> Result<FatDir<'a, DATA>> {
        let entry = self.find(name)?.ok_or(FatError::EntryNotFound)?;

        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }

        // Subdirectories always use cluster chains, never fixed root
        Ok(FatDir {
            data: self.data,
            cluster: entry.cluster(),
            fixed_root: None,
        })
    }

    /// Open a file for reading by name.
    ///
    /// Returns an error if the entry is not found or is a directory.
    pub fn open_file(&self, name: &str) -> Result<FileReader<'a, DATA>> {
        let entry = self.find(name)?.ok_or(FatError::EntryNotFound)?;
        FileReader::new(self.data, &entry)
    }
}

pub struct FatDirIter<'a, DATA: Read + Seek> {
    data: &'a FatFs<DATA>,
    /// Current cluster (or 0 for fixed root directory)
    cluster: Cluster,
    /// Offset within current cluster (or within fixed root dir)
    offset: usize,
    /// For fixed root directory: remaining bytes to read (None for cluster-based)
    fixed_root_remaining: Option<usize>,
    /// For fixed root directory: start byte offset
    fixed_root_start: Option<usize>,
    #[cfg(feature = "lfn")]
    lfn_builder: LfnBuilder,
    /// Buffered cluster data (reduces seeks by reading entire cluster at once)
    #[cfg(feature = "alloc")]
    cluster_buffer: Option<alloc::vec::Vec<u8>>,
    /// Whether the buffer is valid for the current cluster
    #[cfg(feature = "alloc")]
    buffer_valid: bool,
}

impl<DATA: Read + Seek> Iterator for FatDirIter<'_, DATA> {
    type Item = Result<DirectoryEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut data = self.data.data.lock();
        let entry_size = size_of::<RawDirectoryEntry>();
        let cluster_size = data.cluster_size;

        loop {
            // Check bounds and handle cluster transitions
            if let Some(ref mut remaining) = self.fixed_root_remaining {
                // Fixed root directory (FAT12/16)
                if *remaining < entry_size {
                    return None; // End of fixed root directory
                }
            } else {
                // Cluster-based directory (FAT32 or subdirectory)
                // Check if we need to move to the next cluster
                if self.offset >= cluster_size {
                    let next = match self.data.fat.next_cluster(data.deref_mut(), self.cluster.0) {
                        Ok(n) => n,
                        Err(e) => return Some(Err(e)),
                    };
                    match next {
                        Some(cluster) => {
                            self.cluster.0 = cluster as usize;
                            self.offset = 0;
                            #[cfg(feature = "alloc")]
                            {
                                self.buffer_valid = false;
                            }
                        }
                        None => return None, // End of directory
                    }
                }
            }

            // Read the entry - use buffering when alloc is available
            #[cfg(feature = "alloc")]
            let raw_entry = {
                // Ensure buffer is filled
                if !self.buffer_valid || self.cluster_buffer.is_none() {
                    let buffer_size = if self.fixed_root_remaining.is_some() {
                        // For fixed root, buffer the remaining bytes (up to a reasonable size)
                        self.fixed_root_remaining.unwrap().min(4096)
                    } else {
                        cluster_size
                    };

                    let seek_pos = if self.fixed_root_remaining.is_some() {
                        let start = self.fixed_root_start.unwrap();
                        start as u64
                    } else {
                        self.cluster
                            .to_bytes(self.data.info.data_start, cluster_size) as u64
                    };

                    if let Err(e) = data.seek(SeekFrom::Start(seek_pos)) {
                        return Some(Err(FatError::Io(e)));
                    }

                    let mut buffer = alloc::vec![0u8; buffer_size];
                    if let Err(e) = data.read_exact(&mut buffer) {
                        return Some(Err(FatError::Io(e)));
                    }

                    self.cluster_buffer = Some(buffer);
                    self.buffer_valid = true;
                }

                // Read entry from buffer
                let buffer = self.cluster_buffer.as_ref().unwrap();
                let offset = if self.fixed_root_remaining.is_some() {
                    self.offset
                } else {
                    self.offset
                };

                if offset + entry_size > buffer.len() {
                    // Buffer exhausted, need to handle this case
                    // For fixed root: we're done
                    // For cluster-based: handled by cluster transition above
                    if self.fixed_root_remaining.is_some() {
                        return None;
                    }
                    continue;
                }

                let entry_bytes: [u8; 32] = buffer[offset..offset + entry_size]
                    .try_into()
                    .unwrap();

                // Safety: RawDirectoryEntry is a union of properly aligned types
                // and entry_bytes has the correct size
                unsafe { core::mem::transmute::<[u8; 32], RawDirectoryEntry>(entry_bytes) }
            };

            #[cfg(not(feature = "alloc"))]
            let raw_entry = {
                // Calculate seek position
                let seek_pos = if self.fixed_root_remaining.is_some() {
                    let start = self.fixed_root_start.unwrap();
                    (start + self.offset) as u64
                } else {
                    self.cluster
                        .to_bytes(self.data.info.data_start, cluster_size) as u64
                        + self.offset as u64
                };

                if let Err(e) = data.seek(SeekFrom::Start(seek_pos)) {
                    return Some(Err(FatError::Io(e)));
                }

                // Read the directory entry
                match data.read_struct::<RawDirectoryEntry>() {
                    Ok(e) => e,
                    Err(e) => return Some(Err(FatError::Io(e))),
                }
            };

            let entry_bytes = unsafe { raw_entry.bytes };

            // Check for end of directory
            if entry_bytes[0] == 0 {
                #[cfg(feature = "lfn")]
                self.lfn_builder.reset();
                return None;
            }

            // Check for deleted entry
            if entry_bytes[0] == 0xE5 {
                self.offset += entry_size;
                if let Some(ref mut remaining) = self.fixed_root_remaining {
                    *remaining = remaining.saturating_sub(entry_size);
                }
                #[cfg(feature = "lfn")]
                self.lfn_builder.reset(); // Deleted entry breaks LFN sequence
                continue;
            }

            self.offset += entry_size;
            if let Some(ref mut remaining) = self.fixed_root_remaining {
                *remaining = remaining.saturating_sub(entry_size);
            }

            // Check if this is an LFN entry (attributes == LONG_NAME)
            #[cfg(feature = "lfn")]
            {
                let entry_attr = unsafe { raw_entry.file }.attributes;
                if entry_attr == DirEntryAttrFlags::LONG_NAME.bits() {
                    // This is an LFN entry
                    let lfn = unsafe { raw_entry.lfn };
                    let seq = lfn.sequence_number;

                    // Check if this is the start of a new LFN sequence (has 0x40 bit set)
                    if seq & LfnBuilder::LAST_ENTRY_MASK != 0 {
                        self.lfn_builder.start(seq, lfn.checksum);
                    }

                    if self.lfn_builder.building {
                        self.lfn_builder.add_entry(
                            seq,
                            lfn.checksum,
                            &lfn.name1,
                            &lfn.name2,
                            &lfn.name3,
                        );
                    }
                    continue;
                }
            }

            // This is a regular file/directory entry
            let file_entry = unsafe { raw_entry.file };

            // Convert 0x05 back to 0xE5 for kanji compatibility
            let mut name_bytes = file_entry.name;
            if name_bytes[0] == 0x05 {
                name_bytes[0] = 0xE5;
            }

            let short_name = match ShortFileName::new(name_bytes) {
                Ok(n) => n,
                Err(_) => return Some(Err(FatError::InvalidShortFilename)),
            };

            // Try to get the LFN if we've been building one
            #[cfg(feature = "lfn")]
            let long_name = self.lfn_builder.finish(&short_name);

            // For FAT12/16 with fixed root dir, parent_clus is 0 (sentinel)
            // For cluster-based dirs, parent_clus is the actual cluster
            return Some(Ok(DirectoryEntry::Entry(FileEntry {
                short_name,
                #[cfg(feature = "lfn")]
                long_name,
                attr: DirEntryAttrFlags::from_bits_retain(file_entry.attributes),
                size: file_entry.size.get() as usize,
                parent_clus: self.cluster,
                offset_within_cluster: self.offset - entry_size,
                cluster: Cluster::from_parts(
                    file_entry.first_cluster_high.get(),
                    file_entry.first_cluster_low.get(),
                ),
            })));
        }
    }
}

// FileReader is now implemented in read.rs

#[derive(Debug)]
pub enum DirectoryEntry {
    /// A file or directory entry
    Entry(FileEntry),
}

impl DirectoryEntry {
    /// Get the display name of the entry.
    /// Returns the long filename if available, otherwise the short name.
    pub fn name(&self) -> &str {
        match self {
            Self::Entry(ent) => ent.name(),
        }
    }

    /// Get the file entry if this is an Entry variant
    pub fn as_entry(&self) -> Option<&FileEntry> {
        match self {
            Self::Entry(ent) => Some(ent),
        }
    }
}

#[derive(Debug)]
pub struct ParseInfo<T> {
    pub data: T,
    pub warnings: FileSystemWarnings,
    pub errors: FileSystemErrors,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemWarnings: u64 {

    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct FileSystemErrors: u64 {

    }
}

#[derive(Debug)]
pub struct FileEntry {
    pub(crate) short_name: ShortFileName,
    #[cfg(feature = "lfn")]
    pub(crate) long_name: Option<LongFileName>,
    pub(crate) attr: DirEntryAttrFlags,
    pub(crate) size: usize,
    /// Parent directory cluster (used for write operations)
    #[cfg_attr(not(feature = "write"), allow(dead_code))]
    pub(crate) parent_clus: Cluster<usize>,
    /// Offset of this entry within the parent cluster (used for write operations)
    #[cfg_attr(not(feature = "write"), allow(dead_code))]
    pub(crate) offset_within_cluster: usize,
    pub(crate) cluster: Cluster<usize>,
}

impl FileEntry {
    /// Get the file's display name.
    /// Returns the long filename if available, otherwise the short name.
    pub fn name(&self) -> &str {
        #[cfg(feature = "lfn")]
        if let Some(ref lfn) = self.long_name {
            return lfn.as_str();
        }
        self.short_name.as_str()
    }

    /// Get the short (8.3) filename
    pub fn short_name(&self) -> &ShortFileName {
        &self.short_name
    }

    /// Get the long filename, if available
    #[cfg(feature = "lfn")]
    pub fn long_name(&self) -> Option<&LongFileName> {
        self.long_name.as_ref()
    }

    /// Get the file attributes
    pub fn attributes(&self) -> DirEntryAttrFlags {
        self.attr
    }

    /// Check if this entry is a directory
    pub fn is_directory(&self) -> bool {
        self.attr.contains(DirEntryAttrFlags::DIRECTORY)
    }

    /// Check if this entry is a regular file
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    /// Get the file size in bytes (0 for directories)
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the first cluster of the file data
    pub fn cluster(&self) -> Cluster<usize> {
        self.cluster
    }
}

pub enum Fat {
    Fat12(Fat12),
    Fat16(Fat16),
    Fat32(Fat32),
}

impl Fat {
    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        match self {
            Self::Fat12(fat12) => fat12.next_cluster(reader, cluster),
            Self::Fat16(fat16) => fat16.next_cluster(reader, cluster),
            Self::Fat32(fat32) => fat32.next_cluster(reader, cluster),
        }
    }

    /// Get the FAT type for informational purposes
    pub fn fat_type(&self) -> FatType {
        match self {
            Self::Fat12(_) => FatType::Fat12,
            Self::Fat16(_) => FatType::Fat16,
            Self::Fat32(_) => FatType::Fat32,
        }
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn truncate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
    ) -> Result<u32> {
        match self {
            Self::Fat12(fat12) => fat12.truncate_chain(rw, cluster as u16),
            Self::Fat16(fat16) => fat16.truncate_chain(rw, cluster as u16),
            Self::Fat32(fat32) => fat32.truncate_chain(rw, cluster as u32),
        }
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub fn free_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
    ) -> Result<u32> {
        match self {
            Self::Fat12(fat12) => fat12.free_chain(rw, cluster as u16),
            Self::Fat16(fat16) => fat16.free_chain(rw, cluster as u16),
            Self::Fat32(fat32) => fat32.free_chain(rw, cluster as u32),
        }
    }
}

/// FAT filesystem type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

/// FAT12 table implementation.
///
/// FAT12 uses 12-bit entries packed into 3 bytes for every 2 clusters.
pub struct Fat12 {
    start: usize,
    size: usize,
    #[allow(dead_code)]
    count: usize,
    max_cluster: u16,
}

impl Fat12 {
    /// Mask for the 12-bit cluster number
    const ENTRY_MASK: u16 = 0x0FFF;
    /// End of chain markers: 0x0FF8 - 0x0FFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u16 = 0x0FF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u16 = 0x0FF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self { start, size, count, max_cluster }
    }

    /// Calculate byte offset for a FAT12 entry.
    /// FAT12 packs 2 entries into 3 bytes: entry N starts at byte (N * 3) / 2
    fn entry_byte_offset(&self, cluster: usize) -> usize {
        self.start + (cluster * 3) / 2
    }

    fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        let byte_offset = self.entry_byte_offset(cluster);
        reader.seek(SeekFrom::Start(byte_offset as u64))?;

        let mut bytes = [0u8; 2];
        reader.read_exact(&mut bytes)?;

        // FAT12 entry layout:
        // If cluster N is even: entry = (bytes[1] & 0x0F) << 8 | bytes[0]
        // If cluster N is odd:  entry = bytes[1] << 4 | (bytes[0] >> 4)
        let value = if cluster % 2 == 0 {
            u16::from(bytes[0]) | (u16::from(bytes[1] & 0x0F) << 8)
        } else {
            (u16::from(bytes[0]) >> 4) | (u16::from(bytes[1]) << 4)
        };

        Ok(value)
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let entry = self.read_clus(reader, cluster)? & Self::ENTRY_MASK;

        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster { cluster: cluster as u32 });
        }

        self.validate_cluster(entry)?;

        Ok(Some(entry as u32))
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = 0x0FF8;

    /// Write a FAT12 entry at the specified cluster index to a specific FAT copy.
    #[cfg(feature = "write")]
    fn write_clus_at<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let byte_offset = self.start + fat_index * self.size + (cluster * 3) / 2;
        rw.seek(SeekFrom::Start(byte_offset as u64))?;

        // Read existing bytes (we need to preserve the other half)
        let mut bytes = [0u8; 2];
        rw.read_exact(&mut bytes)?;

        // Modify the appropriate bits
        if cluster % 2 == 0 {
            // Even: modify lower 8 bits of bytes[0] and lower 4 bits of bytes[1]
            bytes[0] = value as u8;
            bytes[1] = (bytes[1] & 0xF0) | ((value >> 8) as u8 & 0x0F);
        } else {
            // Odd: modify upper 4 bits of bytes[0] and all of bytes[1]
            bytes[0] = (bytes[0] & 0x0F) | ((value << 4) as u8);
            bytes[1] = (value >> 4) as u8;
        }

        // Write back
        rw.seek(SeekFrom::Start(byte_offset as u64))?;
        rw.write_all(&bytes)?;

        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub fn write_clus<T: Read + Write + Seek>(&self, rw: &mut T, cluster: usize, value: u16) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(rw, cluster, value, i)?;
        }
        Ok(())
    }

    /// Allocate a single cluster, returns the allocated cluster number.
    #[cfg(feature = "write")]
    pub fn allocate_cluster<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        hint: u16,
    ) -> Result<u16> {
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize)? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize)? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        Err(FatError::NoFreeSpace)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u16) -> Result<u32> {
        let mut count = 0u32;
        let mut current = start;

        loop {
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            let next = self.read_clus(rw, current as usize)? & Self::ENTRY_MASK;
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER)?;
            count += 1;

            if Self::is_end_of_chain(next) || Self::is_bad_cluster(next) || next == Self::FREE_CLUSTER {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let next = self.read_clus(rw, cluster as usize)? & Self::ENTRY_MASK;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next) && next >= Self::FIRST_DATA_CLUSTER && next <= self.max_cluster {
            self.free_chain(rw, next)
        } else {
            Ok(0)
        }
    }
}

/// FAT16 table implementation.
pub struct Fat16 {
    start: usize,
    size: usize,
    #[allow(dead_code)]
    count: usize,
    max_cluster: u16,
}

impl Fat16 {
    /// End of chain markers: 0xFFF8 - 0xFFFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u16 = 0xFFF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u16 = 0xFFF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u16 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u16) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self { start, size, count, max_cluster }
    }

    fn entry_offset(&self, cluster: usize) -> usize {
        debug_assert!(cluster * size_of::<u16>() < self.size);
        self.start + cluster * size_of::<u16>()
    }

    fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u16> {
        reader.seek(SeekFrom::Start(self.entry_offset(cluster) as u64))?;
        let mut data = 0u16;
        reader.read_exact(bytemuck::bytes_of_mut(&mut data))?;
        Ok(u16::from_le(data))
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u16) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u16) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u16) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster: cluster as u32,
                max: self.max_cluster as u32,
            });
        }
        Ok(())
    }

    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        let entry = self.read_clus(reader, cluster)?;

        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster { cluster: cluster as u32 });
        }

        self.validate_cluster(entry)?;

        Ok(Some(entry as u32))
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u16 = 0x0000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u16 = 0xFFF8;

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    fn write_clus_at<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u16,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + cluster * size_of::<u16>();
        writer.seek(SeekFrom::Start(offset as u64))?;
        writer.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub fn write_clus<T: Write + Seek>(&self, writer: &mut T, cluster: usize, value: u16) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(writer, cluster, value, i)?;
        }
        Ok(())
    }

    /// Allocate a single cluster, returns the allocated cluster number.
    #[cfg(feature = "write")]
    pub fn allocate_cluster<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        hint: u16,
    ) -> Result<u16> {
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize)?;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize)?;
            if entry == Self::FREE_CLUSTER {
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        Err(FatError::NoFreeSpace)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u16) -> Result<u32> {
        let mut count = 0u32;
        let mut current = start;

        loop {
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            let next = self.read_clus(rw, current as usize)?;
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER)?;
            count += 1;

            if Self::is_end_of_chain(next) || Self::is_bad_cluster(next) || next == Self::FREE_CLUSTER {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u16) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let next = self.read_clus(rw, cluster as usize)?;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next) && next >= Self::FIRST_DATA_CLUSTER && next <= self.max_cluster {
            self.free_chain(rw, next)
        } else {
            Ok(0)
        }
    }
}

pub struct Fat32 {
    start: usize,
    size: usize,
    #[allow(dead_code)]
    count: usize,
    max_cluster: u32,
}

impl Fat32 {
    /// Mask for the 28-bit cluster number (upper 4 bits are reserved)
    const ENTRY_MASK: u32 = 0x0FFF_FFFF;
    /// End of chain markers: 0x0FFFFFF8 - 0x0FFFFFFF indicate end of cluster chain
    const END_OF_CHAIN_MIN: u32 = 0x0FFF_FFF8;
    /// Bad cluster marker
    const BAD_CLUSTER: u32 = 0x0FFF_FFF7;
    /// First valid data cluster (clusters 0 and 1 are reserved)
    const FIRST_DATA_CLUSTER: u32 = 2;

    pub fn new(start: usize, size: usize, count: usize, max_cluster: u32) -> Self {
        debug_assert!(count == 1 || count == 2);
        Self { start, size, count, max_cluster }
    }

    fn entry_offset(&self, cluster: usize) -> usize {
        debug_assert!(cluster * size_of::<u32>() < self.size);
        self.start + cluster * size_of::<u32>()
    }

    fn read_clus<T: Read + Seek>(&self, reader: &mut T, cluster: usize) -> Result<u32> {
        reader.seek(SeekFrom::Start(self.entry_offset(cluster) as u64))?;
        let mut data = 0u32;
        reader.read_exact(bytemuck::bytes_of_mut(&mut data))?;
        Ok(data)
    }

    /// Check if a cluster value represents end-of-chain
    fn is_end_of_chain(value: u32) -> bool {
        value >= Self::END_OF_CHAIN_MIN
    }

    /// Check if a cluster value represents a bad cluster
    fn is_bad_cluster(value: u32) -> bool {
        value == Self::BAD_CLUSTER
    }

    /// Validate that a cluster number is within bounds
    fn validate_cluster(&self, cluster: u32) -> Result<()> {
        if cluster < Self::FIRST_DATA_CLUSTER {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        if cluster > self.max_cluster {
            return Err(FatError::ClusterOutOfBounds {
                cluster,
                max: self.max_cluster,
            });
        }
        Ok(())
    }

    pub fn next_cluster<T: Read + Seek>(
        &self,
        reader: &mut T,
        cluster: usize,
    ) -> Result<Option<u32>> {
        // Read the FAT entry for this cluster
        let raw_entry = self.read_clus(reader, cluster)?;
        let entry = raw_entry & Self::ENTRY_MASK;

        // Check for end of chain
        if Self::is_end_of_chain(entry) {
            return Ok(None);
        }

        // Check for bad cluster
        if Self::is_bad_cluster(entry) {
            return Err(FatError::BadCluster { cluster: cluster as u32 });
        }

        // Validate the next cluster is in bounds
        self.validate_cluster(entry)?;

        Ok(Some(entry))
    }

    /// Write a cluster entry to the FAT table at the specified FAT copy
    #[cfg(feature = "write")]
    fn write_clus_at<T: Write + Seek>(
        &self,
        writer: &mut T,
        cluster: usize,
        value: u32,
        fat_index: usize,
    ) -> Result<()> {
        let offset = self.start + fat_index * self.size + cluster * size_of::<u32>();
        writer.seek(SeekFrom::Start(offset as u64))?;
        writer.write_all(&value.to_le_bytes())?;
        Ok(())
    }

    /// Write a cluster entry to all FAT table copies
    #[cfg(feature = "write")]
    pub fn write_clus<T: Write + Seek>(&self, writer: &mut T, cluster: usize, value: u32) -> Result<()> {
        for i in 0..self.count {
            self.write_clus_at(writer, cluster, value, i)?;
        }
        Ok(())
    }

    /// Free cluster marker
    #[cfg(feature = "write")]
    const FREE_CLUSTER: u32 = 0x00000000;
    /// End of chain marker
    #[cfg(feature = "write")]
    const END_OF_CHAIN: u32 = 0x0FFFFFF8;

    /// Allocate a single cluster, returns the allocated cluster number.
    /// Searches starting from `hint` for a free cluster.
    #[cfg(feature = "write")]
    pub fn allocate_cluster<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        hint: u32,
    ) -> Result<u32> {
        // Start searching from hint, wrapping around if needed
        let start = if hint >= Self::FIRST_DATA_CLUSTER && hint <= self.max_cluster {
            hint
        } else {
            Self::FIRST_DATA_CLUSTER
        };

        // Search from hint to max_cluster
        for cluster in start..=self.max_cluster {
            let entry = self.read_clus(rw, cluster as usize)? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                // Mark as end of chain
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        // Wrap around: search from first cluster to hint
        for cluster in Self::FIRST_DATA_CLUSTER..start {
            let entry = self.read_clus(rw, cluster as usize)? & Self::ENTRY_MASK;
            if entry == Self::FREE_CLUSTER {
                // Mark as end of chain
                self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;
                return Ok(cluster);
            }
        }

        Err(FatError::NoFreeSpace)
    }

    /// Allocate a chain of clusters, linking them together.
    /// Returns the first cluster of the allocated chain.
    #[cfg(feature = "write")]
    pub fn allocate_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        count: usize,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Err(FatError::NoFreeSpace);
        }

        let first = self.allocate_cluster(rw, hint)?;
        let mut prev = first;

        for _ in 1..count {
            let next = self.allocate_cluster(rw, prev + 1)?;
            // Link previous cluster to this one
            self.write_clus(rw, prev as usize, next)?;
            prev = next;
        }

        Ok(first)
    }

    /// Free a cluster chain starting at `start`, returns count of freed clusters.
    #[cfg(feature = "write")]
    pub fn free_chain<T: Read + Write + Seek>(&self, rw: &mut T, start: u32) -> Result<u32> {
        let mut count = 0;
        let mut current = start;

        loop {
            // Validate cluster
            if current < Self::FIRST_DATA_CLUSTER || current > self.max_cluster {
                break;
            }

            // Read the next cluster before freeing
            let raw_entry = self.read_clus(rw, current as usize)?;
            let next = raw_entry & Self::ENTRY_MASK;

            // Free this cluster
            self.write_clus(rw, current as usize, Self::FREE_CLUSTER)?;
            count += 1;

            // Check if this was the end of chain
            if Self::is_end_of_chain(next) || Self::is_bad_cluster(next) || next == Self::FREE_CLUSTER {
                break;
            }

            current = next;
        }

        Ok(count)
    }

    /// Truncate a cluster chain after the specified cluster.
    ///
    /// The specified cluster becomes the end of chain (marked with end-of-chain marker).
    /// All clusters following it are freed.
    ///
    /// Returns the number of clusters freed.
    #[cfg(feature = "write")]
    pub fn truncate_chain<T: Read + Write + Seek>(&self, rw: &mut T, cluster: u32) -> Result<u32> {
        if cluster < Self::FIRST_DATA_CLUSTER || cluster > self.max_cluster {
            return Ok(0);
        }

        // Read the next cluster in chain
        let raw_entry = self.read_clus(rw, cluster as usize)?;
        let next = raw_entry & Self::ENTRY_MASK;

        // Mark this cluster as end of chain
        self.write_clus(rw, cluster as usize, Self::END_OF_CHAIN)?;

        // Free the rest of the chain if there is one
        if !Self::is_end_of_chain(next) && next >= Self::FIRST_DATA_CLUSTER && next <= self.max_cluster {
            self.free_chain(rw, next)
        } else {
            Ok(0)
        }
    }

    /// Extend a cluster chain by appending new clusters.
    /// Returns the first cluster of the newly allocated portion.
    #[cfg(feature = "write")]
    pub fn extend_chain<T: Read + Write + Seek>(
        &self,
        rw: &mut T,
        last: u32,
        count: usize,
        hint: u32,
    ) -> Result<u32> {
        if count == 0 {
            return Ok(last);
        }

        let first_new = self.allocate_chain(rw, count, hint)?;
        // Link the last cluster of existing chain to the new chain
        self.write_clus(rw, last as usize, first_new)?;
        Ok(first_new)
    }
}

/// The RawBpb struct represents the boot sector of any FAT partition
///
/// This only contains the common fields of the boot sector, and is not meant to be used directly
/// for reading or writing to the boot sector, for that, see `RawBootSector`, which contains
/// the boot sector and the extended boot sector
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawBpb {
    /// BS_jmpBoot
    pub jump: [u8; 3],
    /// BS_OEMName
    /// The name of the program that formatted the partition
    pub oem_name: [u8; 8],
    /// BPB_BytsPerSec
    /// The number of bytes per sector
    pub bytes_per_sector: U16<LittleEndian>,
    /// BPB_SecPerClus
    /// The number of sectors per cluster
    pub sectors_per_cluster: u8,
    /// BPB_RsvdSecCnt
    ///
    /// The number of reserved sectors, should be nonzero, ans should be a multiple of the sectors per cluster
    /// This is used to:
    /// 1. align the start of the filesystem to the sectors per cluster
    /// 2. Move the data (cluster 2) to the end of fat tables, so that the data can be read from the start of the filesystem
    pub reserved_sector_count: U16<LittleEndian>,
    /// BPB_NumFATs
    ///
    /// The number of fats, 1 is acceptable, but 2 is recommended
    pub fat_count: u8,
    /// BPB_RootEntCnt
    ///
    /// The number of root directory entries
    /// For FAT32, this should be 0
    /// For FAT12/16, this value multiplied by 32 should be a multiple of the bytes per sector
    /// For FAT16, it is recommended to set this to 512 for maximum compatibility
    pub root_entry_count: [u8; 2],
    /// BPB_TotSec16
    ///
    /// The number of sectors
    /// For FAT32, this should be 0
    /// For FAT16, if the number of sectors is greater than 0x10000, you should use total_sectors_32
    pub total_sectors_16: [u8; 2],
    /// BPB_Media
    ///
    /// See the MediaType enum for more information
    pub media_type: u8,
    /// BPB_FATSz16
    ///
    /// The number of sectors per fat
    /// For FAT32, this should be 0
    pub sectors_per_fat_16: [u8; 2],
    /// BPB_SecPerTrk
    ///
    /// The number of sectors per track
    /// This is only relevant for media with have a geometry and used by BIOS interrupt 0x13
    pub sectors_per_track: [u8; 2],
    /// BPB_NumHeads
    ///
    /// Similar situation as sectors_per_track
    pub num_heads: [u8; 2],
    /// BPB_HiddSec
    ///
    /// The number of hidden sectors predicing the partition that contains the FAT volume.
    /// This must be 0 on media that isn't partitioned
    pub hidden_sector_count: [u8; 4],
    /// BPB_TotSec32
    ///
    /// The total number of sectors for FAT32
    /// For FAT16 use, see total_sectors_16
    pub total_sectors_32: [u8; 4],
}

// Safety: RawBpb is a C-repr struct with no padding issues for its fields
unsafe impl bytemuck::NoUninit for RawBpb {}
unsafe impl bytemuck::Zeroable for RawBpb {}
unsafe impl bytemuck::AnyBitPattern for RawBpb {}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawBpbExt16 {
    /// BS_DrvNum
    pub drive_number: u8,
    /// BS_Reserved1
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the strings "FAT12   ","FAT16   ", or "FAT     "
    pub fs_type: [u8; 8],
    /// Zeros
    /// To make it compatible with bytemuck, instead of using [u8; 448], we use 256 + 128 + 64
    pub padding1: [u8; 448],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawBpbExt32 {
    /// BPB_FatSz32
    ///
    /// The number of sectors per fat
    /// BPB_FATSz16 must be 0
    pub sectors_per_fat_32: U32<LittleEndian>,
    /// BPB_ExtFlags
    ///
    /// See the BpbExt32Flags struct for more information
    pub ext_flags: [u8; 2],
    /// BPB_FSVer
    ///
    /// The version of the file system
    /// This must be set to 0x00
    pub version: [u8; 2],
    /// BPB_RootClus
    ///
    /// The cluster number of the root directory
    /// This should be 2, or the first usable (not bad) cluster usable
    pub root_cluster: U32<LittleEndian>,
    /// BPB_FSInfo
    ///
    /// The sector number of the FSINFO structure
    /// NOTE: There is a copy of the FSINFO structure in the
    /// sequence of backup boot sectors, but only the copy
    /// pointed to by this field is kept up to date (i.e., both the
    /// primary and backup boot record point to the same
    /// FSINFO sector)
    pub fs_info_sector: U16<LittleEndian>,
    /// BPB_BkBootSec
    ///
    /// The sector number of the backup boot sector
    /// If set to 6 (only valid non-zero value), the boot sector
    /// in the reserved area is used to store the backup boot sector
    pub boot_sector: [u8; 2],
    /// BPB_Reserved
    /// Reserved, should be zero
    pub reserved: [u8; 12],
    /// BS_DrvNum
    ///
    /// The BIOS interrupt 0x13 drive number
    /// Should be 0x80 or 0x00
    pub drive_number: u8,
    /// BS_Reserved1
    /// Reserved, should be zero
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the string "FAT32   "
    pub fs_type: [u8; 8],
    /// Zeros
    pub padding1: [u8; 420],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: U16<LittleEndian>,
}

// Safety: RawBpbExt32 is a C-repr struct with no padding issues
unsafe impl bytemuck::NoUninit for RawBpbExt32 {}
unsafe impl bytemuck::Zeroable for RawBpbExt32 {}
unsafe impl bytemuck::AnyBitPattern for RawBpbExt32 {}

/// BPB_ExtFlags
///
/// This is a union of the flags that are set in the BPB_ExtFlags field
/// The flags are the following:
/// bits 0-3: zero based index of the active FAT, mirroring must be disabled
/// bits 4-6: reserved
/// bit 7: FAT mirroring is enabled
/// bits 8-15: reserved
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpbExt32Flags(u16);

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileEntry {
    /// DIR_Name
    ///
    /// The name of the file, padded with spaces, and in the 8.3 format
    /// A value of 0xE5 indicates that the directory is free. For kanji, 0x05 is used instead of 0xE5
    /// The special value 0x00 also indicates that the directory is free, but also all the entries
    /// following it are free
    /// The name cannot start with a space
    /// Only upper case letters, digits, and the following characters are allowed:
    /// $ % ' - _ @ ~ ` ! ( ) { } ^ # &
    pub name: [u8; 11],
    /// DIR_Attr
    ///
    /// The file attributes
    pub attributes: u8,
    /// DIR_NTRes
    ///
    /// Reserved for use by Windows NT
    pub reserved: u8,
    /// DIR_CrtTimeTenth
    ///
    /// The creation time, in tenths of a second
    pub creation_time_tenth: u8,
    /// DIR_CrtTime
    ///
    /// The creation time, granularity is 2 seconds
    pub creation_time: [u8; 2],
    /// DIR_CrtDate
    ///
    /// The creation date
    pub creation_date: [u8; 2],
    /// DIR_LstAccDate
    ///
    /// The last access date
    pub last_access_date: [u8; 2],
    /// DIR_FstClusHI
    ///
    /// The high word of the first cluster number
    pub first_cluster_high: U16<LittleEndian>,
    /// DIR_WrtTime
    ///
    /// The last write time, granularity is 2 seconds
    pub last_write_time: [u8; 2],
    /// DIR_WrtDate
    ///
    /// The last write date
    pub last_write_date: [u8; 2],
    /// DIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: U16<LittleEndian>,
    /// DIR_FileSize
    ///
    /// The size of the file, in bytes
    pub size: U32<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawFileEntry {}
unsafe impl bytemuck::Zeroable for RawFileEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileEntry {}

/// A long file name entry
/// The maximum length of a long file name is 255 characters, not including the null terminator
/// The characters allowed extend these characters:
///  . + , ; = [ ]
/// Embedded paces are also allowed
/// The name is stored in UTF-16 encoding (UNICODE)
/// When the unicode character cannot be translated to ANSI, an underscore is used
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawLfnEntry {
    /// LFN_Ord
    ///
    /// The order of the LFN entry, the contents must be masked with 0x40 for the last entry
    pub sequence_number: u8,
    /// LFN_Name1
    ///
    /// The first part of the long file name
    pub name1: [u8; 10],
    /// LDIR_Attr
    ///
    /// Attributes, must be set to: ATTR_LONG_NAME, which is:
    /// ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID
    pub attributes: u8,
    /// LFN_Type
    ///
    /// The type of the LFN entry, must be set to 0
    pub ty: u8,
    /// LFN_Chksum
    ///
    /// Checksum of name in the associated short name directory entry at the end of the LFN sequence
    /// THe algorithm described in the FAT spec is:
    /// unsigned char ChkSum (unsigned char \*pFcbName)
    /// {
    ///     short FcbNameLen;
    ///     unsigned char Sum;
    ///     Sum = 0;
    ///     for (FcbNameLen=11; FcbNameLen!=0; FcbNameLen--) {
    ///         // NOTE: The operation is an unsigned char rotate right
    ///         Sum = ((Sum & 1) ? 0x80 : 0) + (Sum >> 1) + *pFcbName++;
    ///     }
    ///     return (Sum);
    /// }
    pub checksum: u8,
    /// LFN_Name2
    ///
    /// The second part of the long file name
    pub name2: [u8; 12],
    /// LDIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: [u8; 2],
    /// LFN_Name3
    ///
    /// The third part of the long file name
    pub name3: [u8; 4],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union RawDirectoryEntry {
    pub file: RawFileEntry,
    #[cfg(feature = "lfn")]
    pub lfn: RawLfnEntry,
    pub bytes: [u8; 32],
}

impl RawDirectoryEntry {
    pub fn attributes(&self) -> u8 {
        unsafe { self.file }.attributes
    }
}

// Bytemuck implementations for RawDirectoryEntry union
// These are needed for read_struct to work
unsafe impl bytemuck::NoUninit for RawDirectoryEntry {}
unsafe impl bytemuck::Zeroable for RawDirectoryEntry {}
unsafe impl bytemuck::AnyBitPattern for RawDirectoryEntry {}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFsInfo {
    /// FSI_LeadSig
    ///
    /// The lead signature, this have to be 0x41615252, or 'RRaA'
    pub signature: [u8; 4],
    /// FSI_Reserved1
    pub reserved1: [u8; 480],
    /// FSI_StrucSig
    ///
    /// The structure signature, this have to be 0x61417272, or 'rrAa'
    pub structure_signature: [u8; 4],
    /// FSI_Free_Count
    ///
    /// The number of free clusters, this have to be bigger than 0, and less than or equal to the
    /// total number of clusters
    /// This should remove any used clusters for headers, FAT tables, etc...
    pub free_count: U32<LittleEndian>,
    /// FSI_Nxt_Free
    ///
    /// The next free cluster number, this have to be bigger than 2, and less than or equal to the
    pub next_free: U32<LittleEndian>,
    /// FSI_Reserved2
    pub reserved2: [u8; 12],
    /// FSI_TrailSig
    ///
    /// The trail signature, this have to be 0xAA550000
    pub trail_signature: U32<LittleEndian>,
}

// Safety: RawFsInfo is a C-repr packed struct
unsafe impl bytemuck::NoUninit for RawFsInfo {}
unsafe impl bytemuck::Zeroable for RawFsInfo {}
unsafe impl bytemuck::AnyBitPattern for RawFsInfo {}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirEntryAttrFlags: u8 {
        const READ_ONLY = 1 << 0;
        const HIDDEN = 1 << 1;
        const SYSTEM = 1 << 2;
        const VOLUME_ID = 1 << 3;
        const DIRECTORY = 1 << 4;
        const ARCHIVE = 1 << 5;
    }
}

impl DirEntryAttrFlags {
    pub const LONG_NAME: Self = Self::from_bits_truncate(
        Self::READ_ONLY.bits() | Self::HIDDEN.bits() | Self::SYSTEM.bits() | Self::VOLUME_ID.bits(),
    );
}

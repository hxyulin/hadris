//! exFAT Filesystem implementation.
//!
//! The main entry point for working with exFAT filesystems.

use hadris_common::types::endian::Endian;
use spin::Mutex;

use crate::error::{FatError, Result};
use crate::io::{Read, ReadExt, Seek, SeekFrom, SectorCursor};
#[cfg(feature = "write")]
use crate::io::Write;

use super::bitmap::AllocationBitmap;
use super::boot::{ExFatBootSector, ExFatInfo};
use super::dir::ExFatDir;
use super::entry::{entry_type, ExFatFileEntry, RawDirectoryEntry};
use super::fat::ExFatTable;
use super::file::ExFatFileReader;
use super::upcase::UpcaseTable;

/// exFAT filesystem handle.
///
/// This is the main type for interacting with an exFAT filesystem.
pub struct ExFatFs<DATA: Seek> {
    /// The underlying data source wrapped in a mutex for thread safety
    data: Mutex<SectorCursor<DATA>>,
    /// Computed filesystem information
    info: ExFatInfo,
    /// Allocation bitmap for tracking cluster usage
    bitmap: AllocationBitmap,
    /// FAT table for fragmented files
    fat: ExFatTable,
    /// Up-case table for case-insensitive comparisons
    upcase: UpcaseTable,
    /// First cluster of root directory
    root_cluster: u32,
    /// Whether the root directory is contiguous
    root_contiguous: bool,
    /// Size of the root directory
    root_size: u64,
}

impl<DATA> ExFatFs<DATA>
where
    DATA: Read + Seek,
{
    /// Open an exFAT filesystem from a data source.
    pub fn open(mut data: DATA) -> Result<Self> {
        // Read and validate the boot sector
        let boot = ExFatBootSector::read(&mut data)?;
        let info = boot.info().clone();

        // Create sector cursor
        let cursor = SectorCursor::new(data, info.bytes_per_sector, info.bytes_per_cluster);
        let data = Mutex::new(cursor);

        // Create FAT accessor
        let fat = ExFatTable::new(&info);

        // Create placeholder bitmap and upcase table
        let mut bitmap = AllocationBitmap::new(0, 0, info.cluster_count, true);
        let mut upcase = UpcaseTable::new();

        // Scan root directory for system entries
        let root_cluster = info.root_cluster;
        let root_contiguous = true; // Assume contiguous initially
        let root_size = 0u64;

        {
            let mut guard = data.lock();

            // Read root directory entries to find bitmap and upcase table
            let mut offset = info.cluster_to_offset(root_cluster);
            let mut entries_read = 0;
            const MAX_SYSTEM_ENTRIES: usize = 100;

            while entries_read < MAX_SYSTEM_ENTRIES {
                guard.seek(SeekFrom::Start(offset))?;

                let entry: RawDirectoryEntry = guard.data.read_struct()?;
                let entry_type_byte = unsafe { entry.entry_type };

                if entry_type_byte == entry_type::END_OF_DIRECTORY {
                    break;
                }

                match entry_type_byte {
                    entry_type::ALLOCATION_BITMAP => {
                        let bitmap_entry = unsafe { &entry.bitmap };
                        bitmap = AllocationBitmap::new(
                            bitmap_entry.first_cluster.get(),
                            bitmap_entry.data_length.get(),
                            info.cluster_count,
                            true, // Assume contiguous
                        );
                    }
                    entry_type::UPCASE_TABLE => {
                        let upcase_entry = unsafe { &entry.upcase };
                        // We'll load the upcase table after this loop
                        let first_cluster = upcase_entry.first_cluster.get();
                        let size = upcase_entry.data_length.get();

                        // Load immediately since we have the guard
                        drop(guard);
                        let mut guard2 = data.lock();
                        upcase.load(
                            &mut guard2.data,
                            &info,
                            first_cluster,
                            size,
                            true, // Assume contiguous
                        )?;
                        guard = guard2;
                    }
                    _ => {}
                }

                offset += 32; // Each entry is 32 bytes
                entries_read += 1;
            }

            // Load the allocation bitmap
            if bitmap.first_cluster() != 0 {
                bitmap.load(&mut guard.data, &info)?;
            }
        }

        // If upcase table wasn't found or failed to load, use default
        if !upcase.is_valid() {
            upcase = UpcaseTable::create_default();
        }

        Ok(Self {
            data,
            info,
            bitmap,
            fat,
            upcase,
            root_cluster,
            root_contiguous,
            root_size,
        })
    }

    /// Get filesystem information.
    pub fn info(&self) -> &ExFatInfo {
        &self.info
    }

    /// Get the root directory.
    pub fn root_dir(&self) -> ExFatDir<'_, DATA> {
        ExFatDir {
            fs: self,
            first_cluster: self.root_cluster,
            is_contiguous: self.root_contiguous,
            size: self.root_size,
        }
    }

    /// Open a file by path.
    pub fn open_file(&self, path: &str) -> Result<ExFatFileReader<'_, DATA>> {
        let entry = self.open_path(path)?;
        ExFatFileReader::new(self, &entry)
    }

    /// Open a directory by path.
    pub fn open_dir(&self, path: &str) -> Result<ExFatDir<'_, DATA>> {
        let entry = self.open_path(path)?;

        if !entry.is_directory() {
            return Err(FatError::NotADirectory);
        }

        Ok(ExFatDir {
            fs: self,
            first_cluster: entry.first_cluster,
            is_contiguous: entry.no_fat_chain,
            size: entry.data_length,
        })
    }

    /// Open a file or directory by path.
    pub fn open_path(&self, path: &str) -> Result<ExFatFileEntry> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let components: alloc::vec::Vec<&str> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if components.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let mut current_dir = self.root_dir();

        for (i, component) in components.iter().enumerate() {
            let entry = current_dir.find(component)?.ok_or(FatError::EntryNotFound)?;

            if i < components.len() - 1 {
                // Not the last component, must be a directory
                if !entry.is_directory() {
                    return Err(FatError::NotADirectory);
                }
                current_dir = ExFatDir {
                    fs: self,
                    first_cluster: entry.first_cluster,
                    is_contiguous: entry.no_fat_chain,
                    size: entry.data_length,
                };
            } else {
                // Last component, return the entry
                return Ok(entry);
            }
        }

        Err(FatError::EntryNotFound)
    }

    /// Get the next cluster in a chain.
    pub(crate) fn next_cluster(&self, cluster: u32) -> Result<Option<u32>> {
        let mut guard = self.data.lock();
        self.fat.next_cluster(&mut guard.data, cluster)
    }

    /// Read data at a specific offset.
    pub(crate) fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut guard = self.data.lock();
        guard.seek(SeekFrom::Start(offset))?;
        guard.read_exact(buf)?;
        Ok(())
    }

    /// Read a directory entry at a specific offset.
    pub(crate) fn read_entry_at(&self, offset: u64) -> Result<RawDirectoryEntry> {
        let mut guard = self.data.lock();
        guard.seek(SeekFrom::Start(offset))?;
        let entry: RawDirectoryEntry = guard.data.read_struct()?;
        Ok(entry)
    }

    /// Compare two names using the up-case table (case-insensitive).
    pub(crate) fn names_equal(&self, name1: &str, name2: &str) -> Result<bool> {
        Ok(self.upcase.names_equal(name1, name2))
    }

    /// Compute the name hash for a filename.
    pub fn name_hash(&self, name: &str) -> u16 {
        self.upcase.name_hash(name)
    }

    /// Get the number of free clusters.
    pub fn free_cluster_count(&self) -> u32 {
        self.bitmap.free_cluster_count()
    }

    /// Check if a cluster is allocated.
    pub fn is_cluster_allocated(&self, cluster: u32) -> Result<bool> {
        self.bitmap.is_allocated(cluster)
    }

    /// Get the volume serial number.
    pub fn volume_serial(&self) -> u32 {
        self.info.volume_serial
    }
}

#[cfg(feature = "write")]
impl<DATA> ExFatFs<DATA>
where
    DATA: Read + Write + Seek,
{
    /// Write data at a specific offset.
    pub(crate) fn write_at(&self, offset: u64, buf: &[u8]) -> Result<()> {
        let mut guard = self.data.lock();
        guard.seek(SeekFrom::Start(offset))?;
        guard.write_all(buf)?;
        Ok(())
    }

    /// Flush any pending writes.
    pub(crate) fn flush(&self) -> crate::io::Result<()> {
        let mut guard = self.data.lock();
        guard.flush()
    }

    /// Allocate a cluster using the bitmap.
    pub fn allocate_cluster(&self, hint: u32) -> Result<u32> {
        // First, find a free cluster in the bitmap
        let cluster = {
            self.bitmap.find_free_cluster(hint)?
                .ok_or(FatError::NoFreeSpace)?
        };

        // Mark it as allocated in the bitmap
        // Note: This requires mutable access to bitmap, which we'd need to handle
        // with interior mutability in a real implementation

        Ok(cluster)
    }
}

impl<DATA: Seek> core::fmt::Debug for ExFatFs<DATA> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExFatFs")
            .field("info", &self.info)
            .field("root_cluster", &self.root_cluster)
            .finish_non_exhaustive()
    }
}

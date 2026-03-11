//! exFAT Filesystem implementation.
//!
//! The main entry point for working with exFAT filesystems.

use alloc::string::ToString;
use alloc::vec::Vec;

use hadris_common::types::endian::Endian;
use spin::Mutex;

use crate::error::{FatError, Result};
#[cfg(feature = "write")]
use crate::io::Write;
use crate::io::{Read, ReadExt, SectorCursor, Seek, SeekFrom};

use super::bitmap::AllocationBitmap;
use super::boot::{ExFatBootSector, ExFatInfo};
use super::dir::ExFatDir;
#[cfg(feature = "write")]
use super::entry::FileAttributes;
use super::entry::{ExFatFileEntry, RawDirectoryEntry, entry_type};
#[cfg(feature = "write")]
use super::entry_writer::EntrySetBuilder;
use super::fat::ExFatTable;
use super::file::ExFatFileReader;
#[cfg(feature = "write")]
use super::file::ExFatFileWriter;
use super::upcase::UpcaseTable;

/// exFAT filesystem handle.
///
/// This is the main type for interacting with an exFAT filesystem.
pub struct ExFatFs<DATA: Seek> {
    /// The underlying data source wrapped in a mutex for thread safety
    data: Mutex<SectorCursor<DATA>>,
    /// Computed filesystem information
    info: ExFatInfo,
    /// Allocation bitmap for tracking cluster usage (uses Mutex for write support)
    bitmap: Mutex<AllocationBitmap>,
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
            bitmap: Mutex::new(bitmap),
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

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if components.is_empty() {
            return Err(FatError::InvalidPath);
        }

        let mut current_dir = self.root_dir();

        for (i, component) in components.iter().enumerate() {
            let entry = current_dir
                .find(component)?
                .ok_or(FatError::EntryNotFound)?;

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
        self.bitmap.lock().free_cluster_count()
    }

    /// Check if a cluster is allocated.
    pub fn is_cluster_allocated(&self, cluster: u32) -> Result<bool> {
        self.bitmap.lock().is_allocated(cluster)
    }

    /// Get a reference to the upcase table.
    pub(crate) fn upcase(&self) -> &UpcaseTable {
        &self.upcase
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
    pub(crate) fn flush(&self) -> crate::io::IoResult<()> {
        let mut guard = self.data.lock();
        guard.flush()
    }

    /// Allocate a single cluster.
    ///
    /// The cluster is marked as allocated in the bitmap and as end-of-chain in the FAT.
    pub fn allocate_cluster(&self, hint: u32) -> Result<u32> {
        let mut bitmap = self.bitmap.lock();
        let cluster = bitmap
            .find_free_cluster(hint)?
            .ok_or(FatError::NoFreeSpace)?;

        // Mark as allocated in bitmap
        bitmap.set_allocated(cluster, true)?;

        // Mark as end-of-chain in FAT
        let mut data = self.data.lock();
        self.fat
            .write_entry(&mut data.data, cluster, ExFatTable::END_OF_CHAIN)?;

        Ok(cluster)
    }

    /// Allocate contiguous clusters for an exFAT file.
    ///
    /// Returns (first_cluster, is_contiguous). If contiguous allocation fails,
    /// falls back to FAT chain allocation.
    pub fn allocate_clusters(&self, count: u32, hint: u32) -> Result<(u32, bool)> {
        if count == 0 {
            return Ok((0, true));
        }

        let mut bitmap = self.bitmap.lock();

        // Try to allocate contiguously first
        if let Some(first) = bitmap.find_contiguous_free(count, hint)? {
            // Mark all clusters as allocated
            for i in 0..count {
                bitmap.set_allocated(first + i, true)?;
            }
            return Ok((first, true));
        }

        // Fall back to FAT chain allocation
        drop(bitmap); // Release bitmap lock before FAT operations

        let mut data = self.data.lock();
        let first = self.fat.allocate_chain(&mut data.data, count, hint)?;

        // Also mark in bitmap
        drop(data);
        let mut bitmap = self.bitmap.lock();
        let mut current = first;
        let mut data = self.data.lock();
        for _ in 0..count {
            bitmap.set_allocated(current, true)?;
            if let Some(next) = self.fat.next_cluster(&mut data.data, current)? {
                current = next;
            } else {
                break;
            }
        }

        Ok((first, false))
    }

    /// Free clusters.
    ///
    /// If `is_contiguous` is true, frees `count` contiguous clusters starting from `first`.
    /// Otherwise, follows the FAT chain to free all clusters.
    pub fn free_clusters(&self, first: u32, count: u32, is_contiguous: bool) -> Result<()> {
        if first < 2 {
            return Ok(());
        }

        let mut bitmap = self.bitmap.lock();

        if is_contiguous {
            // Free contiguous clusters
            for i in 0..count {
                bitmap.set_allocated(first + i, false)?;
            }
        } else {
            // Follow FAT chain
            let mut current = first;
            let mut data = self.data.lock();

            loop {
                bitmap.set_allocated(current, false)?;
                let next = self.fat.read_entry(&mut data.data, current)?;
                self.fat
                    .write_entry(&mut data.data, current, ExFatTable::FREE_CLUSTER)?;

                if next == ExFatTable::END_OF_CHAIN || next >= ExFatTable::MEDIA_DESCRIPTOR {
                    break;
                }
                current = next;
            }
        }

        Ok(())
    }

    /// Sync the allocation bitmap to disk.
    pub fn sync_bitmap(&self) -> Result<()> {
        let bitmap = self.bitmap.lock();
        let mut data = self.data.lock();
        bitmap.flush(&mut data.data, &self.info)
    }

    /// Find free entry slots in a directory.
    ///
    /// Returns (cluster, offset_within_cluster) for the first slot.
    fn find_free_entry_slots(
        &self,
        dir: &ExFatDir<'_, DATA>,
        slots_needed: usize,
    ) -> Result<(u32, u64)> {
        let cluster_size = self.info.bytes_per_cluster;
        let mut current_cluster = dir.first_cluster;
        let mut consecutive_free = 0;
        let mut first_free_cluster = current_cluster;
        let mut first_free_offset = 0u64;

        loop {
            let cluster_offset = self.info.cluster_to_offset(current_cluster);

            // Scan this cluster for free entries
            for entry_idx in 0..(cluster_size / 32) {
                let offset = cluster_offset + (entry_idx as u64 * 32);
                let entry = self.read_entry_at(offset)?;
                let entry_type_byte = unsafe { entry.entry_type };

                // Check if this is a free entry (0x00 = end, 0x05 = deleted)
                if entry_type_byte == entry_type::END_OF_DIRECTORY
                    || entry_type_byte == entry_type::DELETED_FILE
                    || entry_type_byte == 0x00
                {
                    if consecutive_free == 0 {
                        first_free_cluster = current_cluster;
                        first_free_offset = entry_idx as u64 * 32;
                    }
                    consecutive_free += 1;

                    if consecutive_free >= slots_needed {
                        return Ok((first_free_cluster, first_free_offset));
                    }
                } else {
                    consecutive_free = 0;
                }
            }

            // Move to next cluster
            if dir.is_contiguous {
                current_cluster += 1;
                // Check if we've exceeded the directory size
                if (current_cluster - dir.first_cluster) as u64 * cluster_size as u64 >= dir.size
                    && dir.size > 0
                {
                    break;
                }
            } else {
                match self.next_cluster(current_cluster)? {
                    Some(next) => current_cluster = next,
                    None => break,
                }
            }
        }

        // Need to extend the directory
        // For now, return an error - directory extension is more complex
        Err(FatError::DirectoryFull)
    }

    /// Write a directory entry set to disk.
    fn write_entry_set(
        &self,
        cluster: u32,
        offset_in_cluster: u64,
        entries: &[RawDirectoryEntry],
    ) -> Result<()> {
        let cluster_offset = self.info.cluster_to_offset(cluster);
        let base_offset = cluster_offset + offset_in_cluster;

        for (i, entry) in entries.iter().enumerate() {
            let entry_offset = base_offset + (i as u64 * 32);
            self.write_at(entry_offset, unsafe { &entry.bytes })?;
        }

        Ok(())
    }

    /// Create a new file in the given directory.
    pub fn create_file(&self, parent: &ExFatDir<'_, DATA>, name: &str) -> Result<ExFatFileEntry> {
        // Check if entry already exists
        if parent.find(name)?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Build the entry set
        let builder = EntrySetBuilder::file(name)?;
        let entries = builder.build(&self.upcase);
        let entry_count = entries.len();

        // Find free slots in the directory
        let (slot_cluster, slot_offset) = self.find_free_entry_slots(parent, entry_count)?;

        // Write the entry set
        self.write_entry_set(slot_cluster, slot_offset, &entries)?;

        // Return the new entry
        let now = super::time::ExFatTimestamp::now();
        Ok(ExFatFileEntry {
            name: name.to_string(),
            attributes: FileAttributes::ARCHIVE,
            first_cluster: 0,
            data_length: 0,
            valid_data_length: 0,
            no_fat_chain: true,
            name_hash: self.upcase.name_hash(name),
            created: now.clone(),
            modified: now.clone(),
            accessed: now,
            parent_cluster: slot_cluster,
            entry_offset: self.info.cluster_to_offset(slot_cluster) + slot_offset,
        })
    }

    /// Create a new directory.
    ///
    /// Note: Unlike FAT, exFAT directories don't have . and .. entries.
    pub fn create_dir(
        &self,
        parent: &ExFatDir<'_, DATA>,
        name: &str,
    ) -> Result<ExFatDir<'_, DATA>> {
        // Check if entry already exists
        if parent.find(name)?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Allocate a cluster for the directory contents
        let dir_cluster = self.allocate_cluster(2)?;

        // Zero out the directory cluster
        let cluster_offset = self.info.cluster_to_offset(dir_cluster);
        let zeros = alloc::vec![0u8; self.info.bytes_per_cluster];
        self.write_at(cluster_offset, &zeros)?;

        // Build the entry set
        let builder = EntrySetBuilder::directory(name)?
            .with_cluster(dir_cluster)
            .with_size(0, self.info.bytes_per_cluster as u64)
            .with_contiguous(true);
        let entries = builder.build(&self.upcase);
        let entry_count = entries.len();

        // Find free slots in parent directory
        let (slot_cluster, slot_offset) = self.find_free_entry_slots(parent, entry_count)?;

        // Write the entry set
        self.write_entry_set(slot_cluster, slot_offset, &entries)?;

        Ok(ExFatDir {
            fs: self,
            first_cluster: dir_cluster,
            is_contiguous: true,
            size: self.info.bytes_per_cluster as u64,
        })
    }

    /// Delete a file or empty directory.
    pub fn delete(&self, entry: &ExFatFileEntry) -> Result<()> {
        // If it's a directory, check if it's empty
        if entry.is_directory() {
            let dir = ExFatDir {
                fs: self,
                first_cluster: entry.first_cluster,
                is_contiguous: entry.no_fat_chain,
                size: entry.data_length,
            };

            // Check for any entries in the directory
            for item in dir.entries() {
                let _ = item?;
                return Err(FatError::DirectoryNotEmpty);
            }
        }

        // Free the cluster chain if there is one
        if entry.first_cluster >= 2 {
            let cluster_count = if entry.no_fat_chain {
                let cluster_size = self.info.bytes_per_cluster as u64;
                ((entry.data_length + cluster_size - 1) / cluster_size) as u32
            } else {
                0 // Will follow FAT chain
            };

            self.free_clusters(entry.first_cluster, cluster_count, entry.no_fat_chain)?;
        }

        // Mark the directory entry as deleted (0x05)
        let deleted_marker = [entry_type::DELETED_FILE];
        self.write_at(entry.entry_offset, &deleted_marker)?;

        Ok(())
    }

    /// Open a file for writing.
    pub fn write_file(&self, entry: &ExFatFileEntry) -> Result<ExFatFileWriter<'_, DATA>> {
        ExFatFileWriter::new(self, entry.clone())
    }

    /// Update the stream extension entry for a file with new size and cluster info.
    ///
    /// Reads the entry set from disk, updates the stream extension's
    /// `valid_data_length`, `data_length`, and `first_cluster` fields,
    /// recalculates the entry set checksum, and writes everything back.
    pub(crate) fn update_entry_size(
        &self,
        entry: &ExFatFileEntry,
        new_valid_data_length: u64,
        new_data_length: u64,
        new_first_cluster: u32,
    ) -> Result<()> {
        use super::entry::{RawStreamExtensionEntry, compute_entry_set_checksum};
        use hadris_common::types::endian::LittleEndian;
        use hadris_common::types::number::{U16, U32, U64};

        let mut guard = self.data.lock();

        // Read the primary (File Directory) entry to get secondary_count
        guard.seek(SeekFrom::Start(entry.entry_offset))?;
        let primary: RawDirectoryEntry = guard.data.read_struct()?;
        let secondary_count = unsafe { primary.file.secondary_count } as usize;

        // Read the full entry set (primary + secondaries)
        let entry_count = 1 + secondary_count;
        let mut entries = Vec::with_capacity(entry_count);
        entries.push(primary);

        for i in 1..entry_count {
            let offset = entry.entry_offset + (i as u64 * 32);
            guard.seek(SeekFrom::Start(offset))?;
            let e: RawDirectoryEntry = guard.data.read_struct()?;
            entries.push(e);
        }

        // Update the stream extension entry (second entry, index 1)
        if entry_count >= 2 {
            let stream = unsafe { &mut entries[1].stream };
            stream.valid_data_length = U64::<LittleEndian>::new(new_valid_data_length);
            stream.data_length = U64::<LittleEndian>::new(new_data_length);
            stream.first_cluster = U32::<LittleEndian>::new(new_first_cluster);
        }

        // Recalculate checksum and update primary entry
        let checksum = compute_entry_set_checksum(&entries);
        unsafe {
            entries[0].file.set_checksum = U16::<LittleEndian>::new(checksum);
        }

        // Write the updated entry set back
        for (i, e) in entries.iter().enumerate() {
            let offset = entry.entry_offset + (i as u64 * 32);
            guard.seek(SeekFrom::Start(offset))?;
            guard.write_all(unsafe { &e.bytes })?;
        }

        Ok(())
    }

    /// Truncate a file to the specified size.
    pub fn truncate(&self, entry: &ExFatFileEntry, new_size: u64) -> Result<()> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        if new_size >= entry.valid_data_length {
            return Ok(()); // Nothing to do
        }

        let cluster_size = self.info.bytes_per_cluster as u64;

        if new_size == 0 {
            // Free all clusters
            if entry.first_cluster >= 2 {
                let cluster_count = if entry.no_fat_chain {
                    ((entry.data_length + cluster_size - 1) / cluster_size) as u32
                } else {
                    0
                };
                self.free_clusters(entry.first_cluster, cluster_count, entry.no_fat_chain)?;
            }
            self.update_entry_size(entry, 0, 0, 0)?;
        } else {
            // Calculate clusters to keep
            let clusters_to_keep = (new_size + cluster_size - 1) / cluster_size;
            let new_data_length = clusters_to_keep * cluster_size;

            if entry.no_fat_chain {
                // For contiguous files, just free the excess clusters
                let total_clusters = (entry.data_length + cluster_size - 1) / cluster_size;
                let clusters_to_free = total_clusters - clusters_to_keep;

                if clusters_to_free > 0 {
                    let first_to_free = entry.first_cluster + clusters_to_keep as u32;
                    self.free_clusters(first_to_free, clusters_to_free as u32, true)?;
                }
            } else {
                // For fragmented files, walk the chain and truncate
                let mut current = entry.first_cluster;
                for _ in 1..clusters_to_keep {
                    let mut data = self.data.lock();
                    if let Some(next) = self.fat.next_cluster(&mut data.data, current)? {
                        current = next;
                    } else {
                        break;
                    }
                }

                // Truncate after this cluster
                let mut data = self.data.lock();
                self.fat.truncate_chain(&mut data.data, current)?;
            }
            self.update_entry_size(entry, new_size, new_data_length, entry.first_cluster)?;
        }

        Ok(())
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

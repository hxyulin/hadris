//! Write operations for FAT filesystems.

#[cfg(feature = "write")]
use core::ops::DerefMut;

#[cfg(feature = "write")]
use crate::{
    DirEntryAttrFlags, Fat, FatDir, FatFs, FileEntry, RawDirectoryEntry, RawFileEntry,
    error::{FatError, Result},
    file::ShortFileName,
    io::{Cluster, ClusterLike, Read, ReadExt, Seek, SeekFrom, Write},
};

#[cfg(feature = "write")]
use hadris_common::types::endian::{Endian, LittleEndian};

/// FAT date/time representation for directory entries.
#[cfg(feature = "write")]
#[derive(Debug, Clone, Copy)]
pub struct FatDateTime {
    /// Date: (year-1980)<<9 | month<<5 | day
    pub date: u16,
    /// Time: hour<<11 | minute<<5 | (second/2)
    pub time: u16,
    /// 10ms units (0-199) for creation time
    pub time_tenth: u8,
}

#[cfg(feature = "write")]
impl FatDateTime {
    /// Create a FatDateTime from components.
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        let year_offset = year.saturating_sub(1980).min(127);
        let date = (year_offset << 9) | ((month as u16 & 0x0F) << 5) | (day as u16 & 0x1F);
        let time = ((hour as u16 & 0x1F) << 11)
            | ((minute as u16 & 0x3F) << 5)
            | ((second as u16 / 2) & 0x1F);
        Self {
            date,
            time,
            time_tenth: 0,
        }
    }

    /// Get current date/time (requires std feature).
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        use chrono::{Datelike, Local, Timelike};
        let now = Local::now();
        let year = now.year() as u16;
        let month = now.month() as u8;
        let day = now.day() as u8;
        let hour = now.hour() as u8;
        let minute = now.minute() as u8;
        let second = now.second() as u8;
        let millis = now.timestamp_subsec_millis();

        let mut dt = Self::new(year, month, day, hour, minute, second);
        // time_tenth is in 10ms units (0-199): (second % 2) * 100 + (millis / 10)
        dt.time_tenth = ((second % 2) as u32 * 100 + millis / 10).min(199) as u8;
        dt
    }

    /// Fallback for no-std: returns epoch (Jan 1, 1980 00:00:00).
    #[cfg(not(feature = "std"))]
    pub fn now() -> Self {
        Self::new(1980, 1, 1, 0, 0, 0)
    }

    /// Convert to raw bytes for directory entry.
    pub fn to_raw(&self) -> (u16, u16, u8) {
        (self.date, self.time, self.time_tenth)
    }
}

#[cfg(feature = "write")]
impl Default for FatDateTime {
    fn default() -> Self {
        Self::now()
    }
}

/// A writer for file content in a FAT filesystem.
#[cfg(feature = "write")]
pub struct FileWriter<'a, DATA: Read + Write + Seek> {
    fs: &'a FatFs<DATA>,
    /// First cluster of the file (None if empty file)
    first_cluster: Option<Cluster<usize>>,
    /// Current cluster being written to
    current_cluster: Option<Cluster<usize>>,
    /// Offset within the current cluster
    offset_in_cluster: usize,
    /// Total bytes written so far
    total_written: usize,
    /// Parent directory cluster (0 for fixed root directory)
    entry_parent: Cluster<usize>,
    /// Offset of the directory entry within the parent
    entry_offset: usize,
    /// Fixed root directory info (for FAT12/16)
    fixed_root: Option<(usize, usize)>,
}

#[cfg(feature = "write")]
impl<'a, DATA: Read + Write + Seek> FileWriter<'a, DATA> {
    /// Create a new FileWriter for a file entry.
    ///
    /// The entry must be a file (not a directory).
    pub fn new(fs: &'a FatFs<DATA>, entry: &FileEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        let first_cluster = if entry.cluster().0 >= 2 {
            Some(entry.cluster())
        } else {
            None
        };

        // Get fixed root info if the parent is the root directory (cluster 0)
        // and this is a FAT12/16 filesystem
        let fixed_root = if entry.parent_clus.0 == 0 {
            fs.fixed_root_dir_info()
        } else {
            None
        };

        Ok(Self {
            fs,
            first_cluster,
            current_cluster: first_cluster,
            offset_in_cluster: 0,
            total_written: 0,
            entry_parent: entry.parent_clus,
            entry_offset: entry.offset_within_cluster,
            fixed_root,
        })
    }

    /// Write data to the file.
    ///
    /// Allocates new clusters as needed.
    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut data = self.fs.data.lock();
        let cluster_size = data.cluster_size;
        let mut written = 0;

        while written < buf.len() {
            // Check if we need a new cluster
            if self.current_cluster.is_none() || self.offset_in_cluster >= cluster_size {
                // Need to allocate a new cluster
                let hint = self.current_cluster.map(|c| c.0 as u32 + 1).unwrap_or(2);
                let new_cluster = match &self.fs.fat {
                    Fat::Fat12(fat12) => {
                        fat12.allocate_cluster(data.deref_mut(), hint as u16)? as u32
                    }
                    Fat::Fat16(fat16) => {
                        fat16.allocate_cluster(data.deref_mut(), hint as u16)? as u32
                    }
                    Fat::Fat32(fat32) => fat32.allocate_cluster(data.deref_mut(), hint)?,
                };

                // Update FSInfo tracking (FAT32 only)
                self.fs.decrement_free_count();
                self.fs.update_next_free_hint(new_cluster);

                // Link previous cluster to the new one
                if let Some(prev) = self.current_cluster {
                    match &self.fs.fat {
                        Fat::Fat12(fat12) => {
                            fat12.write_clus(data.deref_mut(), prev.0, new_cluster as u16)?;
                        }
                        Fat::Fat16(fat16) => {
                            fat16.write_clus(data.deref_mut(), prev.0, new_cluster as u16)?;
                        }
                        Fat::Fat32(fat32) => {
                            fat32.write_clus(data.deref_mut(), prev.0, new_cluster)?;
                        }
                    }
                }

                // Update first cluster if this is the first allocation
                if self.first_cluster.is_none() {
                    self.first_cluster = Some(Cluster(new_cluster as usize));
                }

                self.current_cluster = Some(Cluster(new_cluster as usize));
                self.offset_in_cluster = 0;
            }

            let cluster = self.current_cluster.unwrap();
            let bytes_left_in_cluster = cluster_size - self.offset_in_cluster;
            let to_write = (buf.len() - written).min(bytes_left_in_cluster);

            // Seek to the correct position
            let seek_pos =
                cluster.to_bytes(self.fs.info.data_start, cluster_size) + self.offset_in_cluster;
            data.seek(SeekFrom::Start(seek_pos as u64))?;

            // Write the data
            data.write_all(&buf[written..written + to_write])?;

            self.offset_in_cluster += to_write;
            self.total_written += to_write;
            written += to_write;
        }

        Ok(written)
    }

    /// Get the total number of bytes written.
    pub fn bytes_written(&self) -> usize {
        self.total_written
    }

    /// Finish writing and update the directory entry with the new size.
    ///
    /// This must be called after writing to persist the file size.
    pub fn finish(self) -> Result<()> {
        let mut data = self.fs.data.lock();
        let cluster_size = data.cluster_size;

        // Calculate entry position - handle fixed root directory
        let entry_pos = if self.entry_parent.0 == 0 {
            // Fixed root directory (FAT12/16)
            let (root_start, _) = self.fixed_root.expect("Fixed root info required");
            root_start + self.entry_offset
        } else {
            // Cluster-based directory
            self.entry_parent
                .to_bytes(self.fs.info.data_start, cluster_size)
                + self.entry_offset
        };

        // Read the current directory entry
        data.seek(SeekFrom::Start(entry_pos as u64))?;

        let mut raw_entry = data.read_struct::<RawDirectoryEntry>()?;
        let file_entry = unsafe { &mut raw_entry.file };

        // Update size
        file_entry.size =
            hadris_common::types::number::U32::<LittleEndian>::new(self.total_written as u32);

        // Update first cluster - for FAT12/16, only use low 16 bits
        if let Some(cluster) = self.first_cluster {
            let (high, low) = match &self.fs.fat {
                Fat::Fat12(_) | Fat::Fat16(_) => (0u16, cluster.0 as u16),
                Fat::Fat32(_) => ((cluster.0 >> 16) as u16, cluster.0 as u16),
            };
            file_entry.first_cluster_high =
                hadris_common::types::number::U16::<LittleEndian>::new(high);
            file_entry.first_cluster_low =
                hadris_common::types::number::U16::<LittleEndian>::new(low);
        } else {
            file_entry.first_cluster_high =
                hadris_common::types::number::U16::<LittleEndian>::new(0);
            file_entry.first_cluster_low =
                hadris_common::types::number::U16::<LittleEndian>::new(0);
        }

        // Update write time
        let now = FatDateTime::now();
        file_entry.last_write_date = now.date.to_le_bytes();
        file_entry.last_write_time = now.time.to_le_bytes();
        file_entry.last_access_date = now.date.to_le_bytes();

        // Write back the entry
        data.seek(SeekFrom::Start(entry_pos as u64))?;
        data.write_all(bytemuck::bytes_of(&raw_entry))?;
        data.flush()?;

        Ok(())
    }
}

/// Extension trait for FatFs to write files.
#[cfg(feature = "write")]
pub trait FatFsWriteExt<DATA: Read + Write + Seek> {
    /// Create a writer for a file entry.
    fn write_file<'a>(&'a self, entry: &FileEntry) -> Result<FileWriter<'a, DATA>>;

    /// Truncate a file to the specified size.
    ///
    /// If `new_size` is greater than or equal to the current file size, this method
    /// does nothing. Otherwise, it frees any clusters that are no longer needed
    /// and updates the directory entry with the new size.
    ///
    /// # Errors
    ///
    /// Returns [`FatError::NotAFile`] if the entry is a directory.
    fn truncate(&self, entry: &FileEntry, new_size: usize) -> Result<()>;
}

#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFsWriteExt<DATA> for FatFs<DATA> {
    fn write_file<'a>(&'a self, entry: &FileEntry) -> Result<FileWriter<'a, DATA>> {
        FileWriter::new(self, entry)
    }

    fn truncate(&self, entry: &FileEntry, new_size: usize) -> Result<()> {
        if !entry.is_file() {
            return Err(FatError::NotAFile);
        }

        let current_size = entry.size();
        if new_size >= current_size {
            return Ok(()); // Nothing to do
        }

        let first_cluster = entry.cluster();
        let cluster_size = self.info.cluster_size;

        // Get fixed root info if the parent is in the fixed root directory
        let fixed_root = if entry.parent_clus.0 == 0 {
            self.fixed_root_dir_info()
        } else {
            None
        };

        if new_size == 0 {
            // Free entire chain
            let freed_count = if first_cluster.0 >= 2 {
                let mut data = self.data.lock();
                self.fat.free_chain(data.deref_mut(), first_cluster.0)?
            } else {
                0
            };
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);
            // Update directory entry: size=0, first_cluster=0
            self.update_entry_size_and_cluster(entry, 0, Cluster(0), fixed_root)?;
        } else {
            // Calculate which cluster to keep
            let clusters_needed = new_size.div_ceil(cluster_size);

            // Walk chain to find the last cluster to keep
            let mut current = first_cluster;
            for _ in 1..clusters_needed {
                let mut data = self.data.lock();
                if let Some(next) = self.fat.next_cluster(data.deref_mut(), current.0)? {
                    current = Cluster(next as usize);
                } else {
                    break;
                }
            }

            // Truncate after this cluster
            let freed_count = {
                let mut data = self.data.lock();
                self.fat.truncate_chain(data.deref_mut(), current.0)?
            };
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);

            // Update directory entry with new size (keep first_cluster)
            self.update_entry_size_and_cluster(entry, new_size, first_cluster, fixed_root)?;
        }

        Ok(())
    }
}

/// Directory write operations
#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFs<DATA> {
    /// Find a free entry slot in a directory.
    ///
    /// For fixed root directories (FAT12/16), searches the fixed area and returns
    /// DirectoryFull if full since it cannot be expanded.
    ///
    /// For cluster-based directories, searches the cluster chain and allocates
    /// a new cluster if needed.
    fn find_free_entry_slot_in_dir(
        &self,
        dir: &FatDir<'_, DATA>,
    ) -> Result<(Cluster<usize>, usize)> {
        if let Some((root_start, root_size)) = dir.fixed_root {
            // Fixed root directory (FAT12/16) - cannot be expanded
            self.find_free_entry_in_fixed_root(root_start, root_size)
        } else {
            // Cluster-based directory
            self.find_free_entry_in_cluster_chain(dir.cluster)
        }
    }

    /// Find a free entry in a fixed root directory (FAT12/16).
    ///
    /// Returns DirectoryFull if the fixed root directory is full.
    fn find_free_entry_in_fixed_root(
        &self,
        root_start: usize,
        root_size: usize,
    ) -> Result<(Cluster<usize>, usize)> {
        let mut data = self.data.lock();
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let max_entries = root_size / entry_size;

        for i in 0..max_entries {
            let offset = i * entry_size;
            let seek_pos = root_start + offset;
            data.seek(SeekFrom::Start(seek_pos as u64))?;

            let raw_entry = data.read_struct::<RawDirectoryEntry>()?;
            let first_byte = unsafe { raw_entry.bytes[0] };

            // 0x00 = free entry and all following are free
            // 0xE5 = deleted entry (free)
            if first_byte == 0x00 || first_byte == 0xE5 {
                // Return Cluster(0) as sentinel for fixed root
                return Ok((Cluster(0), offset));
            }
        }

        // Root directory is full and cannot be expanded
        Err(FatError::DirectoryFull)
    }

    /// Find a free entry slot in a cluster-based directory.
    ///
    /// If no free slot is found in the existing chain, allocates a new cluster.
    fn find_free_entry_in_cluster_chain(
        &self,
        dir_cluster: Cluster<usize>,
    ) -> Result<(Cluster<usize>, usize)> {
        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let entries_per_cluster = cluster_size / entry_size;
        let mut current_cluster = dir_cluster;

        loop {
            // Search this cluster for a free entry
            for i in 0..entries_per_cluster {
                let offset = i * entry_size;
                let seek_pos =
                    current_cluster.to_bytes(self.info.data_start, cluster_size) + offset;
                data.seek(SeekFrom::Start(seek_pos as u64))?;

                let raw_entry = data.read_struct::<RawDirectoryEntry>()?;
                let first_byte = unsafe { raw_entry.bytes[0] };

                // 0x00 = free entry and all following are free
                // 0xE5 = deleted entry (free)
                if first_byte == 0x00 || first_byte == 0xE5 {
                    return Ok((current_cluster, offset));
                }
            }

            // Try to get next cluster
            let next = self.fat.next_cluster(data.deref_mut(), current_cluster.0)?;
            match next {
                Some(cluster) => {
                    current_cluster = Cluster(cluster as usize);
                }
                None => {
                    // No more clusters, need to allocate a new one
                    let new_cluster = match &self.fat {
                        Fat::Fat12(fat12) => {
                            let hint = (current_cluster.0 as u16).saturating_add(1);
                            let new = fat12.allocate_cluster(data.deref_mut(), hint)?;
                            // Link the last cluster to the new one
                            fat12.write_clus(data.deref_mut(), current_cluster.0, new)?;
                            new as u32
                        }
                        Fat::Fat16(fat16) => {
                            let hint = (current_cluster.0 as u16).saturating_add(1);
                            let new = fat16.allocate_cluster(data.deref_mut(), hint)?;
                            // Link the last cluster to the new one
                            fat16.write_clus(data.deref_mut(), current_cluster.0, new)?;
                            new as u32
                        }
                        Fat::Fat32(fat32) => {
                            let hint = current_cluster.0 as u32 + 1;
                            let new = fat32.allocate_cluster(data.deref_mut(), hint)?;
                            // Link the last cluster to the new one
                            fat32.write_clus(data.deref_mut(), current_cluster.0, new)?;
                            new
                        }
                    };

                    // Update FSInfo tracking (FAT32 only)
                    self.decrement_free_count();
                    self.update_next_free_hint(new_cluster);

                    // Zero out the new cluster
                    let new_cluster_pos =
                        Cluster(new_cluster as usize).to_bytes(self.info.data_start, cluster_size);
                    data.seek(SeekFrom::Start(new_cluster_pos as u64))?;
                    let zeros = alloc::vec![0u8; cluster_size];
                    data.write_all(&zeros)?;

                    return Ok((Cluster(new_cluster as usize), 0));
                }
            }
        }
    }

    /// Write a raw directory entry at the specified location.
    ///
    /// For fixed root directory entries (cluster == 0), uses the fixed root offset.
    fn write_raw_entry(
        &self,
        cluster: Cluster<usize>,
        offset: usize,
        entry: &RawFileEntry,
        fixed_root: Option<(usize, usize)>,
    ) -> Result<()> {
        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;

        // Calculate seek position
        let seek_pos = if cluster.0 == 0 {
            // Fixed root directory (FAT12/16)
            let (root_start, _) = fixed_root.expect("Fixed root info required for cluster 0");
            root_start + offset
        } else {
            // Cluster-based directory
            cluster.to_bytes(self.info.data_start, cluster_size) + offset
        };

        data.seek(SeekFrom::Start(seek_pos as u64))?;
        data.write_all(bytemuck::bytes_of(entry))?;
        Ok(())
    }

    /// Create a new file in the given directory.
    ///
    /// Returns the FileEntry for the newly created file.
    pub fn create_file(&self, parent: &FatDir<'_, DATA>, name: &str) -> Result<FileEntry> {
        // Check if entry already exists
        if parent.find(name)?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Generate short filename (suffix=0 means no ~N suffix)
        let short_name =
            ShortFileName::from_long_name(name, 0).map_err(|_| FatError::InvalidFilename)?;

        // Find a free slot
        let (slot_cluster, slot_offset) = self.find_free_entry_slot_in_dir(parent)?;

        // Create the directory entry
        let now = FatDateTime::now();
        let (date, time, time_tenth) = now.to_raw();

        let entry = RawFileEntry {
            name: short_name.to_raw_bytes(),
            attributes: DirEntryAttrFlags::ARCHIVE.bits(),
            reserved: 0,
            creation_time_tenth: time_tenth,
            creation_time: time.to_le_bytes(),
            creation_date: date.to_le_bytes(),
            last_access_date: date.to_le_bytes(),
            first_cluster_high: hadris_common::types::number::U16::<LittleEndian>::new(0),
            last_write_time: time.to_le_bytes(),
            last_write_date: date.to_le_bytes(),
            first_cluster_low: hadris_common::types::number::U16::<LittleEndian>::new(0),
            size: hadris_common::types::number::U32::<LittleEndian>::new(0),
        };

        self.write_raw_entry(slot_cluster, slot_offset, &entry, parent.fixed_root)?;

        Ok(FileEntry {
            short_name,
            #[cfg(feature = "lfn")]
            long_name: None,
            attr: DirEntryAttrFlags::ARCHIVE,
            size: 0,
            parent_clus: slot_cluster,
            offset_within_cluster: slot_offset,
            cluster: Cluster(0),
        })
    }

    /// Create a new directory.
    ///
    /// Returns a FatDir handle for the newly created directory.
    pub fn create_dir<'a>(
        &'a self,
        parent: &FatDir<'a, DATA>,
        name: &str,
    ) -> Result<FatDir<'a, DATA>> {
        // Check if entry already exists
        if parent.find(name)?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Generate short filename (suffix=0 means no ~N suffix)
        let short_name =
            ShortFileName::from_long_name(name, 0).map_err(|_| FatError::InvalidFilename)?;

        // Allocate a cluster for the directory contents
        let new_cluster = {
            let mut data = self.data.lock();
            match &self.fat {
                Fat::Fat12(fat12) => fat12.allocate_cluster(data.deref_mut(), 2)? as u32,
                Fat::Fat16(fat16) => fat16.allocate_cluster(data.deref_mut(), 2)? as u32,
                Fat::Fat32(fat32) => fat32.allocate_cluster(data.deref_mut(), 2)?,
            }
        };

        // Update FSInfo tracking (FAT32 only)
        self.decrement_free_count();
        self.update_next_free_hint(new_cluster);

        // Find a free slot in parent
        let (slot_cluster, slot_offset) = self.find_free_entry_slot_in_dir(parent)?;

        // Create the directory entry in parent
        let now = FatDateTime::now();
        let (date, time, time_tenth) = now.to_raw();

        // For FAT12/16, only use the low 16 bits of the cluster number
        let (cluster_high, cluster_low) = match &self.fat {
            Fat::Fat12(_) | Fat::Fat16(_) => (0u16, new_cluster as u16),
            Fat::Fat32(_) => ((new_cluster >> 16) as u16, new_cluster as u16),
        };

        let entry = RawFileEntry {
            name: short_name.to_raw_bytes(),
            attributes: DirEntryAttrFlags::DIRECTORY.bits(),
            reserved: 0,
            creation_time_tenth: time_tenth,
            creation_time: time.to_le_bytes(),
            creation_date: date.to_le_bytes(),
            last_access_date: date.to_le_bytes(),
            first_cluster_high: hadris_common::types::number::U16::<LittleEndian>::new(
                cluster_high,
            ),
            last_write_time: time.to_le_bytes(),
            last_write_date: date.to_le_bytes(),
            first_cluster_low: hadris_common::types::number::U16::<LittleEndian>::new(cluster_low),
            size: hadris_common::types::number::U32::<LittleEndian>::new(0),
        };

        self.write_raw_entry(slot_cluster, slot_offset, &entry, parent.fixed_root)?;

        // Initialize the new directory with . and .. entries
        {
            let mut data = self.data.lock();
            let cluster_size = data.cluster_size;
            let dir_pos =
                Cluster(new_cluster as usize).to_bytes(self.info.data_start, cluster_size);

            // Zero out the cluster first
            data.seek(SeekFrom::Start(dir_pos as u64))?;
            let zeros = alloc::vec![0u8; cluster_size];
            data.write_all(&zeros)?;

            // Write "." entry (points to self)
            let dot_entry = RawFileEntry {
                name: *b".          ",
                attributes: DirEntryAttrFlags::DIRECTORY.bits(),
                reserved: 0,
                creation_time_tenth: time_tenth,
                creation_time: time.to_le_bytes(),
                creation_date: date.to_le_bytes(),
                last_access_date: date.to_le_bytes(),
                first_cluster_high: hadris_common::types::number::U16::<LittleEndian>::new(
                    cluster_high,
                ),
                last_write_time: time.to_le_bytes(),
                last_write_date: date.to_le_bytes(),
                first_cluster_low: hadris_common::types::number::U16::<LittleEndian>::new(
                    cluster_low,
                ),
                size: hadris_common::types::number::U32::<LittleEndian>::new(0),
            };
            data.seek(SeekFrom::Start(dir_pos as u64))?;
            data.write_all(bytemuck::bytes_of(&dot_entry))?;

            // Write ".." entry (points to parent)
            // For FAT12/16 root directory (cluster 0), ".." should point to cluster 0
            // For FAT32 root or any subdirectory, use the parent's cluster
            let parent_cluster = parent.cluster.0 as u32;
            let (parent_high, parent_low) = match &self.fat {
                Fat::Fat12(_) | Fat::Fat16(_) => (0u16, parent_cluster as u16),
                Fat::Fat32(_) => ((parent_cluster >> 16) as u16, parent_cluster as u16),
            };

            let dotdot_entry = RawFileEntry {
                name: *b"..         ",
                attributes: DirEntryAttrFlags::DIRECTORY.bits(),
                reserved: 0,
                creation_time_tenth: time_tenth,
                creation_time: time.to_le_bytes(),
                creation_date: date.to_le_bytes(),
                last_access_date: date.to_le_bytes(),
                first_cluster_high: hadris_common::types::number::U16::<LittleEndian>::new(
                    parent_high,
                ),
                last_write_time: time.to_le_bytes(),
                last_write_date: date.to_le_bytes(),
                first_cluster_low: hadris_common::types::number::U16::<LittleEndian>::new(
                    parent_low,
                ),
                size: hadris_common::types::number::U32::<LittleEndian>::new(0),
            };
            let dotdot_pos = dir_pos + core::mem::size_of::<RawDirectoryEntry>();
            data.seek(SeekFrom::Start(dotdot_pos as u64))?;
            data.write_all(bytemuck::bytes_of(&dotdot_entry))?;
        }

        Ok(FatDir {
            data: self,
            cluster: Cluster(new_cluster as usize),
            fixed_root: None, // Newly created directories are never fixed root
        })
    }

    /// Delete a file or empty directory.
    pub fn delete(&self, entry: &FileEntry) -> Result<()> {
        // If it's a directory, check if it's empty (only . and ..)
        if entry.is_directory() {
            let dir = FatDir {
                data: self,
                cluster: entry.cluster(),
                fixed_root: None, // User-created directories are never fixed root
            };

            let mut count = 0;
            for item in dir.entries() {
                let item = item?;
                let name = item.name();
                if name != "." && name != ".." {
                    count += 1;
                }
            }

            if count > 0 {
                return Err(FatError::DirectoryNotEmpty);
            }
        }

        // Free the cluster chain if there is one
        if entry.cluster().0 >= 2 {
            let freed_count = {
                let mut data = self.data.lock();
                match &self.fat {
                    Fat::Fat12(fat12) => {
                        fat12.free_chain(data.deref_mut(), entry.cluster().0 as u16)?
                    }
                    Fat::Fat16(fat16) => {
                        fat16.free_chain(data.deref_mut(), entry.cluster().0 as u16)?
                    }
                    Fat::Fat32(fat32) => {
                        fat32.free_chain(data.deref_mut(), entry.cluster().0 as u32)?
                    }
                }
            };
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);
        }

        // Mark the directory entry as deleted
        {
            let mut data = self.data.lock();
            let cluster_size = data.cluster_size;

            // Calculate entry position - handle fixed root directory
            let entry_pos = if entry.parent_clus.0 == 0 {
                // Fixed root directory (FAT12/16)
                let (root_start, _) = self
                    .fixed_root_dir_info()
                    .expect("Fixed root info required for cluster 0");
                root_start + entry.offset_within_cluster
            } else {
                // Cluster-based directory
                entry
                    .parent_clus
                    .to_bytes(self.info.data_start, cluster_size)
                    + entry.offset_within_cluster
            };

            data.seek(SeekFrom::Start(entry_pos as u64))?;
            // Write 0xE5 as the first byte to mark as deleted
            data.write_all(&[0xE5])?;
        }

        Ok(())
    }

    /// Update a directory entry's size and first cluster fields.
    ///
    /// This is used by truncate and other operations that need to modify these fields.
    fn update_entry_size_and_cluster(
        &self,
        entry: &FileEntry,
        new_size: usize,
        first_cluster: Cluster<usize>,
        fixed_root: Option<(usize, usize)>,
    ) -> Result<()> {
        use crate::Fat;

        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;

        // Calculate entry position - handle fixed root directory
        let entry_pos = if entry.parent_clus.0 == 0 {
            // Fixed root directory (FAT12/16)
            let (root_start, _) = fixed_root.expect("Fixed root info required for cluster 0");
            root_start + entry.offset_within_cluster
        } else {
            // Cluster-based directory
            entry
                .parent_clus
                .to_bytes(self.info.data_start, cluster_size)
                + entry.offset_within_cluster
        };

        // Read the current directory entry
        data.seek(SeekFrom::Start(entry_pos as u64))?;

        let mut raw_entry = data.read_struct::<RawDirectoryEntry>()?;
        let file_entry = unsafe { &mut raw_entry.file };

        // Update size
        file_entry.size = hadris_common::types::number::U32::<LittleEndian>::new(new_size as u32);

        // Update first cluster
        let (high, low) = if first_cluster.0 >= 2 {
            match &self.fat {
                Fat::Fat12(_) | Fat::Fat16(_) => (0u16, first_cluster.0 as u16),
                Fat::Fat32(_) => ((first_cluster.0 >> 16) as u16, first_cluster.0 as u16),
            }
        } else {
            (0u16, 0u16)
        };
        file_entry.first_cluster_high =
            hadris_common::types::number::U16::<LittleEndian>::new(high);
        file_entry.first_cluster_low = hadris_common::types::number::U16::<LittleEndian>::new(low);

        // Update modification time
        let now = FatDateTime::now();
        file_entry.last_write_date = now.date.to_le_bytes();
        file_entry.last_write_time = now.time.to_le_bytes();
        file_entry.last_access_date = now.date.to_le_bytes();

        // Write back the entry
        data.seek(SeekFrom::Start(entry_pos as u64))?;
        data.write_all(bytemuck::bytes_of(&raw_entry))?;

        Ok(())
    }
}

/// FSInfo update operations
#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFs<DATA> {
    /// Synchronize the FSInfo sector to disk.
    ///
    /// For FAT32 filesystems, this updates the FSInfo sector with the current
    /// free cluster count and next free cluster hint. For FAT12/16 filesystems,
    /// this only flushes pending writes.
    pub fn sync(&self) -> Result<()> {
        self.write_fsinfo()?;

        let mut data = self.data.lock();
        data.flush()?;
        Ok(())
    }

    /// Write the FSInfo sector to disk (FAT32 only).
    ///
    /// This updates the free cluster count and next free cluster hint in the
    /// FSInfo sector. For FAT12/16 filesystems, this is a no-op.
    fn write_fsinfo(&self) -> Result<()> {
        use crate::fs::FatFsExt;
        use crate::raw::RawFsInfo;

        let ext = match &self.ext {
            FatFsExt::Fat32(ext) => ext,
            _ => return Ok(()), // No FSInfo for FAT12/16
        };

        let mut data = self.data.lock();

        // Seek to FSInfo sector
        data.seek_sector(ext.fs_info_sec)?;

        // Read current FSInfo to preserve other fields
        let mut fs_info = data.read_struct::<RawFsInfo>()?;

        // Update the mutable fields
        fs_info.free_count =
            hadris_common::types::number::U32::<LittleEndian>::new(ext.free_count.get());
        fs_info.next_free =
            hadris_common::types::number::U32::<LittleEndian>::new(ext.next_free.get().0);

        // Write back
        data.seek_sector(ext.fs_info_sec)?;
        data.write_all(bytemuck::bytes_of(&fs_info))?;

        Ok(())
    }

    /// Decrement the free cluster count (called after cluster allocation).
    ///
    /// This only affects FAT32 filesystems.
    pub(crate) fn decrement_free_count(&self) {
        use crate::fs::FatFsExt;

        if let FatFsExt::Fat32(ext) = &self.ext {
            let count = ext.free_count.get();
            if count > 0 && count != 0xFFFFFFFF {
                ext.free_count.set(count - 1);
            }
        }
    }

    /// Increment the free cluster count (called after cluster free).
    ///
    /// This only affects FAT32 filesystems.
    pub(crate) fn increment_free_count(&self, amount: u32) {
        use crate::fs::FatFsExt;

        if let FatFsExt::Fat32(ext) = &self.ext {
            let count = ext.free_count.get();
            if count != 0xFFFFFFFF {
                ext.free_count.set(count.saturating_add(amount));
            }
        }
    }

    /// Update the next free cluster hint (called after cluster allocation).
    ///
    /// This only affects FAT32 filesystems.
    pub(crate) fn update_next_free_hint(&self, cluster: u32) {
        use crate::fs::FatFsExt;

        if let FatFsExt::Fat32(ext) = &self.ext {
            // Set hint to the cluster after the one just allocated
            ext.next_free.set(Cluster(cluster.saturating_add(1)));
        }
    }

    /// Get the current free cluster count (FAT32 only).
    ///
    /// Returns `None` for FAT12/16 filesystems or if the count is unknown (0xFFFFFFFF).
    pub fn free_cluster_count(&self) -> Option<u32> {
        use crate::fs::FatFsExt;

        match &self.ext {
            FatFsExt::Fat32(ext) => {
                let count = ext.free_count.get();
                if count != 0xFFFFFFFF {
                    Some(count)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get the next free cluster hint (FAT32 only).
    ///
    /// Returns `None` for FAT12/16 filesystems or if the hint is unknown.
    pub fn next_free_cluster_hint(&self) -> Option<u32> {
        use crate::fs::FatFsExt;

        match &self.ext {
            FatFsExt::Fat32(ext) => {
                let hint = ext.next_free.get().0;
                if hint >= 2 && hint != 0xFFFFFFFF {
                    Some(hint)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

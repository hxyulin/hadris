//! Write operations for FAT filesystems.

io_transform! {

#[cfg(feature = "write")]
use core::ops::DerefMut;

#[cfg(feature = "write")]
use crate::{
    raw::{DirEntryAttrFlags, RawDirectoryEntry, RawFileEntry},
    error::{FatError, Result},
    file::ShortFileName,
};
#[cfg(feature = "write")]
use super::{
    fat_table::Fat, dir::{FatDir, FileEntry}, fs::FatFs,
    io::{Cluster, ClusterLike, Read, ReadExt, Seek, SeekFrom, Write},
};

#[cfg(feature = "write")]
use hadris_common::types::endian::{Endian, LittleEndian};

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
    /// Override for the modified timestamp written by `finish()`. When `None`,
    /// the configured `TimeProvider` supplies "now".
    pending_modified: Option<crate::time::FatDateTime>,
    /// Override for the access date written by `finish()`. FAT stores no
    /// access time, only a date — hence `u16` rather than `FatDateTime`.
    pending_accessed: Option<u16>,
    /// Override for the creation timestamp written by `finish()`.
    pending_created: Option<crate::time::FatDateTime>,
    /// Set to `true` only inside `finish()` once the on-disk entry has been
    /// updated. Drives the `dirty-file-panic` Drop check; when the feature
    /// is off, drop is a no-op regardless of this field.
    finished: bool,
}

#[cfg(feature = "write")]
impl<'a, DATA: Read + Write + Seek> Drop for FileWriter<'a, DATA> {
    fn drop(&mut self) {
        // Without `dirty-file-panic`, drop is a no-op: callers that forget
        // `finish()` silently lose the size/timestamp commit (the data
        // bytes themselves are already on disk because `write()` flushes
        // immediately). With the feature, we panic loudly so the bug is
        // caught in dev rather than in production.
        #[cfg(feature = "dirty-file-panic")]
        if !self.finished {
            panic!(
                "FileWriter dropped without calling finish() — \
                 directory entry size/timestamps are NOT committed. \
                 Disable the `dirty-file-panic` feature if this is intended."
            );
        }
    }
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
            pending_modified: None,
            pending_accessed: None,
            pending_created: None,
            finished: false,
        })
    }

    /// Create a FileWriter positioned at the end of the file for appending.
    ///
    /// Walks the FAT chain to find the last cluster and positions the
    /// writer at the file's current end. Subsequent writes append data
    /// and `finish()` updates the size to include both existing and new data.
    pub async fn new_append(fs: &'a FatFs<DATA>, entry: &FileEntry) -> Result<Self> {
        if entry.is_directory() {
            return Err(FatError::NotAFile);
        }

        let fixed_root = if entry.parent_clus.0 == 0 {
            fs.fixed_root_dir_info()
        } else {
            None
        };

        let file_size = entry.len() as usize;
        let first_cluster = if entry.cluster().0 >= 2 {
            Some(entry.cluster())
        } else {
            None
        };

        if file_size == 0 || first_cluster.is_none() {
            // Empty file — same as a regular new writer
            return Ok(Self {
                fs,
                first_cluster,
                current_cluster: first_cluster,
                offset_in_cluster: 0,
                total_written: 0,
                entry_parent: entry.parent_clus,
                entry_offset: entry.offset_within_cluster,
                fixed_root,
                pending_modified: None,
                pending_accessed: None,
                pending_created: None,
                finished: false,
            });
        }

        let cluster_size = {
            let data = fs.data.lock();
            data.cluster_size
        };

        // Walk the FAT chain to find the last cluster. Bounded by
        // `max_cluster` so a corrupt looping chain cannot hang the writer.
        let max_steps = fs.fat.max_cluster();
        let last = {
            let mut data = fs.data.lock();
            fs.fat
                .walk_chain(data.deref_mut(), first_cluster.unwrap().0 as u32, max_steps, |_| {})
                .await?
        };
        let current = Cluster(last as usize);

        let offset_in_last = file_size % cluster_size;

        Ok(Self {
            fs,
            first_cluster,
            current_cluster: Some(current),
            offset_in_cluster: offset_in_last,
            total_written: file_size,
            entry_parent: entry.parent_clus,
            entry_offset: entry.offset_within_cluster,
            fixed_root,
            pending_modified: None,
            pending_accessed: None,
            pending_created: None,
            finished: false,
        })
    }

    /// Write data to the file.
    ///
    /// Allocates new clusters as needed.
    pub async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let cluster_size = self.fs.info.cluster_size;
        let mut written = 0;

        while written < buf.len() {
            // Check if we need a new cluster
            if self.current_cluster.is_none() || self.offset_in_cluster >= cluster_size {
                // Allocate via the routed helper so the FAT cache (when
                // installed) sees the mutation. The helper acquires both
                // cache+data locks internally in canonical order.
                let hint = self.current_cluster.map(|c| c.0 as u32 + 1).unwrap_or(2);
                let new_cluster = self.fs.allocate_cluster_routed(hint).await?;

                // Update FSInfo tracking (FAT32 only)
                self.fs.decrement_free_count();
                self.fs.update_next_free_hint(new_cluster);

                // Link previous cluster to the new one (also routed).
                if let Some(prev) = self.current_cluster {
                    self.fs.write_clus_routed(prev.0, new_cluster).await?;
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

            // Lock data only for the payload write.
            {
                let mut data = self.fs.data.lock();
                let seek_pos = cluster.to_bytes(self.fs.info.data_start, cluster_size)
                    + self.offset_in_cluster;
                data.seek(SeekFrom::Start(seek_pos as u64)).await?;
                data.write_all(&buf[written..written + to_write]).await?;
            }

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

    /// Override the modified timestamp written by [`finish`](Self::finish).
    ///
    /// Without this call, `finish()` stamps "now" via the configured
    /// [`TimeProvider`](crate::time::TimeProvider). Useful for preserving the
    /// original mtime when copying files between volumes, or for
    /// reproducible-image builds.
    pub fn set_modified(&mut self, dt: crate::time::FatDateTime) -> &mut Self {
        self.pending_modified = Some(dt);
        self
    }

    /// Override the access date written by [`finish`](Self::finish).
    ///
    /// FAT does not store an access *time* — only a date. Pass the raw
    /// FAT-encoded date `(year-1980)<<9 | month<<5 | day`.
    pub fn set_accessed(&mut self, date: u16) -> &mut Self {
        self.pending_accessed = Some(date);
        self
    }

    /// Override the creation timestamp written by [`finish`](Self::finish).
    ///
    /// Most filesystems write creation time only at file-create. This setter
    /// lets writers retroactively patch it, useful when re-imaging or
    /// migrating data with timestamps from another source.
    pub fn set_created(&mut self, dt: crate::time::FatDateTime) -> &mut Self {
        self.pending_created = Some(dt);
        self
    }

    /// Finish writing and update the directory entry with the new size.
    ///
    /// This must be called after writing to persist the file size. On FAT32 it
    /// also flushes the FSInfo sector so its `free_count` matches the FAT —
    /// without this, `fsck.fat` rejects the image after writes.
    ///
    /// With the `dirty-file-panic` feature enabled, dropping the writer
    /// without calling `finish` panics — the most common cause of "the file
    /// I just wrote shows up as zero bytes" bugs.
    pub async fn finish(mut self) -> Result<()> {
        {
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
            data.seek(SeekFrom::Start(entry_pos as u64)).await?;

            let mut raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
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

            // Update timestamps. Overrides win over the configured clock so
            // callers can preserve original times when copying or rebuilding.
            let modified = self.pending_modified.unwrap_or_else(|| self.fs.time_provider().now());
            file_entry.last_write_date = modified.date.to_le_bytes();
            file_entry.last_write_time = modified.time.to_le_bytes();
            file_entry.last_access_date = self
                .pending_accessed
                .unwrap_or(modified.date)
                .to_le_bytes();
            if let Some(created) = self.pending_created {
                file_entry.creation_date = created.date.to_le_bytes();
                file_entry.creation_time = created.time.to_le_bytes();
                file_entry.creation_time_tenth = created.time_tenth;
            }

            // Write back the entry
            data.seek(SeekFrom::Start(entry_pos as u64)).await?;
            data.write_all(bytemuck::bytes_of(&raw_entry)).await?;
            data.flush().await?;
        }

        // Flush FSInfo so on-disk free_count matches in-memory state
        // (no-op for FAT12/16). The lock above must be released first because
        // write_fsinfo also acquires it.
        self.fs.write_fsinfo().await?;

        // Mark as cleanly finished so the Drop guard (under
        // `dirty-file-panic`) accepts the consume.
        self.finished = true;

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
    async fn truncate(&self, entry: &FileEntry, new_size: usize) -> Result<()>;

    /// Patch the timestamps on an existing entry without rewriting its data.
    ///
    /// Each parameter is `Option`: `None` keeps the on-disk value untouched.
    /// `accessed_date` is the raw FAT-encoded date (FAT does not store an
    /// access *time*).
    ///
    /// Useful when copying files between volumes or rebuilding a reproducible
    /// image — every other write path stamps "now" via the configured
    /// [`TimeProvider`](crate::time::TimeProvider), which is the wrong
    /// behaviour for those workflows.
    async fn set_times(
        &self,
        entry: &FileEntry,
        modified: Option<crate::time::FatDateTime>,
        accessed_date: Option<u16>,
        created: Option<crate::time::FatDateTime>,
    ) -> Result<()>;
}

#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFsWriteExt<DATA> for FatFs<DATA> {
    fn write_file<'a>(&'a self, entry: &FileEntry) -> Result<FileWriter<'a, DATA>> {
        FileWriter::new(self, entry)
    }

    async fn truncate(&self, entry: &FileEntry, new_size: usize) -> Result<()> {
        if !entry.is_file() {
            return Err(FatError::NotAFile);
        }

        let current_size = entry.len() as usize;
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
            // Free entire chain — routed through cache when installed.
            let freed_count = if first_cluster.0 >= 2 {
                self.free_chain_routed(first_cluster.0 as u32).await?
            } else {
                0
            };
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);
            // Update directory entry: size=0, first_cluster=0
            self.update_entry_size_and_cluster(entry, 0, Cluster(0), fixed_root).await?;
        } else {
            // Calculate which cluster to keep
            let clusters_needed = new_size.div_ceil(cluster_size);

            // Walk chain to find the last cluster to keep. The hop count is
            // bounded both by the file size and by `max_cluster` so a
            // looping chain on corrupt media surfaces as ClusterLoop.
            let max_cluster = self.fat.max_cluster();
            let hops = ((clusters_needed.saturating_sub(1)) as u32).min(max_cluster);
            let current = {
                let mut steps_remaining = hops;
                let mut cur = first_cluster.0 as u32;
                while steps_remaining > 0 {
                    match self.next_cluster_routed(cur as usize).await? {
                        Some(next) => cur = next,
                        None => break,
                    }
                    steps_remaining -= 1;
                }
                Cluster(cur as usize)
            };

            // Truncate after this cluster — routed.
            let freed_count = self.truncate_chain_routed(current.0 as u32).await?;
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);

            // Update directory entry with new size (keep first_cluster)
            self.update_entry_size_and_cluster(entry, new_size, first_cluster, fixed_root).await?;
        }

        // Flush FSInfo so on-disk free_count matches in-memory state (FAT32).
        self.write_fsinfo().await?;

        Ok(())
    }

    async fn set_times(
        &self,
        entry: &FileEntry,
        modified: Option<crate::time::FatDateTime>,
        accessed_date: Option<u16>,
        created: Option<crate::time::FatDateTime>,
    ) -> Result<()> {
        if modified.is_none() && accessed_date.is_none() && created.is_none() {
            return Ok(());
        }

        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;

        let entry_pos = if entry.parent_clus.0 == 0 {
            let (root_start, _) = self
                .fixed_root_dir_info()
                .expect("Fixed root info required for cluster 0");
            root_start + entry.offset_within_cluster
        } else {
            entry
                .parent_clus
                .to_bytes(self.info.data_start, cluster_size)
                + entry.offset_within_cluster
        };

        data.seek(SeekFrom::Start(entry_pos as u64)).await?;
        let mut raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
        let file_entry = unsafe { &mut raw_entry.file };

        if let Some(m) = modified {
            file_entry.last_write_date = m.date.to_le_bytes();
            file_entry.last_write_time = m.time.to_le_bytes();
        }
        if let Some(date) = accessed_date {
            file_entry.last_access_date = date.to_le_bytes();
        }
        if let Some(c) = created {
            file_entry.creation_date = c.date.to_le_bytes();
            file_entry.creation_time = c.time.to_le_bytes();
            file_entry.creation_time_tenth = c.time_tenth;
        }

        data.seek(SeekFrom::Start(entry_pos as u64)).await?;
        data.write_all(bytemuck::bytes_of(&raw_entry)).await?;
        data.flush().await?;

        Ok(())
    }
}

/// Convert 0xE5 to 0x05 in the first byte of a short name for kanji compatibility.
///
/// The FAT spec uses 0xE5 as a deleted-entry marker, so actual filenames starting
/// with byte 0xE5 (valid kanji lead byte) must be stored as 0x05. The read path
/// converts 0x05 back to 0xE5.
#[cfg(feature = "write")]
fn kanji_short_name_fixup(name: &mut [u8; 11]) {
    if name[0] == 0xE5 {
        name[0] = 0x05;
    }
}

/// Maximum number of LFN entries the spec allows: 20 entries × 13 UTF-16 code
/// units per entry = 260 char "ceiling", though the spec caps the encoded
/// name itself at 255 code units.
#[cfg(all(feature = "write", feature = "lfn"))]
pub(crate) const MAX_LFN_ENTRIES: usize = 20;

/// Decide whether `name` can be stored as a single short (8.3) directory entry
/// using the Windows NT `DIR_NTRes` case flags, and if so which flags to set.
///
/// Returns `Some(bits)` when the name fits 8.3 with at most a per-part *uniform*
/// case difference — `bits` carries `LOWER_BASE` (0x08) and/or `LOWER_EXT`
/// (0x10) so a lowercase name round-trips without a long-file-name entry. An
/// already-uppercase 8.3 name returns `Some(0)`. Returns `None` when the name
/// needs LFN entries to round-trip: too long, spaces, multiple dots, mixed case
/// within the base or extension, or characters not representable in the 8.3
/// character set.
///
/// This replaces the older "does this need an LFN?" predicate — a `None` result
/// is exactly the set of names that previously required LFN entries.
#[cfg(feature = "write")]
fn short_name_case_bits(name: &str) -> Option<u8> {
    const LOWER_BASE: u8 = 0x08;
    const LOWER_EXT: u8 = 0x10;

    let (base, ext) = match name.rfind('.') {
        Some(pos) if pos > 0 => (&name[..pos], &name[pos + 1..]),
        _ => (name, ""),
    };
    if base.is_empty() || base.chars().count() > 8 || ext.chars().count() > 3 {
        return None;
    }
    if name.matches('.').count() > 1 {
        return None;
    }

    // Returns `Some(true)` for an all-lowercase part, `Some(false)` for an
    // all-uppercase (or caseless) part, and `None` when the part is not 8.3
    // representable (invalid character or mixed case).
    fn part_is_lower(part: &str) -> Option<bool> {
        let mut seen_lower = false;
        let mut seen_upper = false;
        for c in part.chars() {
            if !c.is_ascii() {
                return None;
            }
            let upper = (c as u8).to_ascii_uppercase();
            let representable = upper.is_ascii_uppercase()
                || upper.is_ascii_digit()
                || ShortFileName::ALLOWED_SYMBOLS.contains(&upper);
            if !representable {
                return None;
            }
            if c.is_ascii_lowercase() {
                seen_lower = true;
            } else if c.is_ascii_uppercase() {
                seen_upper = true;
            }
        }
        if seen_lower && seen_upper {
            return None;
        }
        Some(seen_lower)
    }

    let mut bits = 0;
    if part_is_lower(base)? {
        bits |= LOWER_BASE;
    }
    if part_is_lower(ext)? {
        bits |= LOWER_EXT;
    }
    Some(bits)
}

/// Maximum number of LFN entries we'll walk backward when cleaning up
/// orphaned long-name slots on delete/rename. The FAT spec caps at 20
/// entries per name; bounding the scan defends against corrupt directory
/// contents that would otherwise spoof an unbounded LFN run.
#[cfg(feature = "write")]
const LFN_CLEANUP_SCAN_LIMIT: usize = 20;

#[cfg(feature = "write")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DirectoryEntryPosition {
    cluster: Cluster<usize>,
    offset: usize,
}

#[cfg(feature = "write")]
const MAX_DIRECTORY_ENTRY_RUN: usize = MAX_LFN_ENTRIES + 1;

#[cfg(feature = "write")]
#[derive(Clone, Copy, Debug)]
struct DirectoryEntryRun {
    positions: [DirectoryEntryPosition; MAX_DIRECTORY_ENTRY_RUN],
    len: usize,
}

#[cfg(feature = "write")]
impl DirectoryEntryRun {
    fn new() -> Self {
        Self {
            positions: [DirectoryEntryPosition {
                cluster: Cluster(0),
                offset: 0,
            }; MAX_DIRECTORY_ENTRY_RUN],
            len: 0,
        }
    }

    fn push(&mut self, position: DirectoryEntryPosition) {
        debug_assert!(self.len < self.positions.len());
        self.positions[self.len] = position;
        self.len += 1;
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn get(&self, index: usize) -> DirectoryEntryPosition {
        debug_assert!(index < self.len);
        self.positions[index]
    }

    fn last(&self) -> DirectoryEntryPosition {
        self.get(self.len - 1)
    }
}

/// Encode `name` (UTF-8) into UTF-16LE LFN entries, written into `out` in
/// disk order. Returns the number of LFN entries produced (excluding the
/// short entry).
///
/// Disk layout placed into `out`:
///   `out[0]`               = sequence N | LAST_ENTRY_MASK (highest seq)
///   `out[1..n]`            = sequences N-1, N-2, ..., 1
///   (caller writes the short entry into `out[n]`)
///
/// Returns `None` if the name exceeds 255 UTF-16 code units (FAT spec cap).
#[cfg(all(feature = "write", feature = "lfn"))]
fn build_lfn_entries(
    name: &str,
    short_checksum: u8,
    out: &mut [RawDirectoryEntry],
) -> Option<usize> {
    use crate::raw::RawLfnEntry;

    // Worst-case staging buffer: 20 LFN entries × 13 UTF-16 units = 260.
    // Sized larger than the spec cap (255) so we always have room for the
    // 0x0000 terminator + 0xFFFF filler when a 255-unit name doesn't
    // perfectly fill the last entry. A 255-unit buffer (the previous size)
    // would index out of bounds at exactly the spec cap.
    const STAGING_CAP: usize = MAX_LFN_ENTRIES * crate::file::LongFileName::CHARS_PER_ENTRY;
    let mut u16_buf = [0u16; STAGING_CAP];
    let mut u16_len = 0usize;
    for ch in name.chars() {
        let mut tmp = [0u16; 2];
        for &c in ch.encode_utf16(&mut tmp).iter() {
            // Cap the *encoded* length at 255 (FAT spec) — anything longer
            // surfaces as `None` so the caller can return `InvalidFilename`.
            if u16_len >= crate::file::LFN_MAX_UTF16_UNITS {
                return None;
            }
            u16_buf[u16_len] = c;
            u16_len += 1;
        }
    }

    let chars_per_entry = crate::file::LongFileName::CHARS_PER_ENTRY;
    let num_lfn = u16_len.div_ceil(chars_per_entry);
    if num_lfn == 0 || num_lfn > MAX_LFN_ENTRIES || out.len() < num_lfn {
        return None;
    }

    // Pad the unused tail of the last entry: 0x0000 terminator immediately
    // after the last real char, then 0xFFFF for any remaining slots — that's
    // what the spec expects from a writer. Skip when the name perfectly
    // fills the last entry (terminator omitted in that case per spec).
    let total_capacity = num_lfn * chars_per_entry;
    if u16_len < total_capacity {
        u16_buf[u16_len] = 0x0000;
        for slot in &mut u16_buf[u16_len + 1..total_capacity] {
            *slot = 0xFFFF;
        }
    }

    // LFN entries on disk are stored in reverse: the first entry encountered
    // by a reader has the highest sequence number (with `LAST_ENTRY_MASK`)
    // and contains the *last* segment of the name. Walk from highest seq
    // down to 1, slotting them into out[0..num_lfn].
    for (entry_idx, out_entry) in out.iter_mut().enumerate().take(num_lfn) {
        let seq_num = (num_lfn - entry_idx) as u8;
        let seq_byte = if entry_idx == 0 {
            seq_num | crate::file::LfnBuilder::LAST_ENTRY_MASK
        } else {
            seq_num
        };

        let chunk_start = (seq_num as usize - 1) * chars_per_entry;
        let chunk = &u16_buf[chunk_start..chunk_start + chars_per_entry];

        let mut name1 = [0u8; 10];
        let mut name2 = [0u8; 12];
        let mut name3 = [0u8; 4];
        for i in 0..5 {
            let bytes = chunk[i].to_le_bytes();
            name1[i * 2] = bytes[0];
            name1[i * 2 + 1] = bytes[1];
        }
        for i in 0..6 {
            let bytes = chunk[5 + i].to_le_bytes();
            name2[i * 2] = bytes[0];
            name2[i * 2 + 1] = bytes[1];
        }
        for i in 0..2 {
            let bytes = chunk[11 + i].to_le_bytes();
            name3[i * 2] = bytes[0];
            name3[i * 2 + 1] = bytes[1];
        }

        let lfn = RawLfnEntry {
            sequence_number: seq_byte,
            name1,
            attributes: DirEntryAttrFlags::LONG_NAME.bits(),
            ty: 0,
            checksum: short_checksum,
            name2,
            first_cluster_low: [0, 0],
            name3,
        };
        *out_entry = RawDirectoryEntry { lfn };
    }

    Some(num_lfn)
}

/// Directory write operations
#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFs<DATA> {
    async fn mark_entry_span_deleted(&self, entry: &FileEntry) -> Result<()> {
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let cluster_size = self.info.cluster_size;
        let target = DirectoryEntryPosition {
            cluster: entry.parent_clus,
            offset: entry.offset_within_cluster,
        };
        let mut pending = DirectoryEntryRun::new();

        if entry.parent_dir_clus.0 == 0 {
            let (root_start, root_size) = self
                .fixed_root_dir_info()
                .expect("Fixed root info required for cluster 0");
            let max_entries = root_size / entry_size;
            let mut data = self.data.lock();
            for i in 0..max_entries {
                let position = DirectoryEntryPosition {
                    cluster: Cluster(0),
                    offset: i * entry_size,
                };
                if position == target {
                    for index in 0..pending.len {
                        data.seek(SeekFrom::Start(
                            (root_start + pending.get(index).offset) as u64,
                        ))
                        .await?;
                        data.write_all(&[0xE5]).await?;
                    }
                    data.seek(SeekFrom::Start((root_start + position.offset) as u64))
                        .await?;
                    data.write_all(&[0xE5]).await?;
                    return Ok(());
                }

                data.seek(SeekFrom::Start((root_start + position.offset) as u64))
                    .await?;
                let raw = data.read_struct::<RawDirectoryEntry>().await?;
                let bytes = unsafe { raw.bytes };
                if bytes[0] == 0x00 {
                    break;
                }
                if bytes[0] != 0xE5
                    && unsafe { raw.file }.attributes == DirEntryAttrFlags::LONG_NAME.bits()
                {
                    if pending.len == LFN_CLEANUP_SCAN_LIMIT {
                        for index in 1..pending.len {
                            pending.positions[index - 1] = pending.positions[index];
                        }
                        pending.len -= 1;
                    }
                    pending.push(position);
                } else {
                    pending.clear();
                }
            }
            return Err(FatError::EntryNotFound);
        }

        let mut current = entry.parent_dir_clus;
        let mut steps = 0u32;
        loop {
            steps = steps.saturating_add(1);
            if steps > self.fat.max_cluster() {
                return Err(FatError::ClusterLoop {
                    cluster: current.0 as u32,
                });
            }

            {
                let mut data = self.data.lock();
                for offset in (0..cluster_size).step_by(entry_size) {
                    let position = DirectoryEntryPosition {
                        cluster: current,
                        offset,
                    };
                    let seek_pos =
                        current.to_bytes(self.info.data_start, cluster_size) + offset;
                    if position == target {
                        for index in 0..pending.len {
                            let previous = pending.get(index);
                            let previous_pos = previous
                                .cluster
                                .to_bytes(self.info.data_start, cluster_size)
                                + previous.offset;
                            data.seek(SeekFrom::Start(previous_pos as u64)).await?;
                            data.write_all(&[0xE5]).await?;
                        }
                        data.seek(SeekFrom::Start(seek_pos as u64)).await?;
                        data.write_all(&[0xE5]).await?;
                        return Ok(());
                    }

                    data.seek(SeekFrom::Start(seek_pos as u64)).await?;
                    let raw = data.read_struct::<RawDirectoryEntry>().await?;
                    let bytes = unsafe { raw.bytes };
                    if bytes[0] == 0x00 {
                        return Err(FatError::EntryNotFound);
                    }
                    if bytes[0] != 0xE5
                        && unsafe { raw.file }.attributes == DirEntryAttrFlags::LONG_NAME.bits()
                    {
                        if pending.len == LFN_CLEANUP_SCAN_LIMIT {
                            for index in 1..pending.len {
                                pending.positions[index - 1] = pending.positions[index];
                            }
                            pending.len -= 1;
                        }
                        pending.push(position);
                    } else {
                        pending.clear();
                    }
                }
            }

            match self.next_cluster_routed(current.0).await? {
                Some(next) => current = Cluster(next as usize),
                None => return Err(FatError::EntryNotFound),
            }
        }
    }

    /// Find `count` consecutive free entry slots in a directory, allocating
    /// new directory clusters if needed.
    ///
    /// The returned positions are in logical directory order and may cross
    /// cluster boundaries.
    async fn find_free_entry_run_in_dir(
        &self,
        dir: &FatDir<'_, DATA>,
        count: usize,
    ) -> Result<DirectoryEntryRun> {
        debug_assert!((1..=MAX_DIRECTORY_ENTRY_RUN).contains(&count));
        if let Some((root_start, root_size)) = dir.fixed_root {
            self.find_free_entry_run_in_fixed_root(root_start, root_size, count)
                .await
        } else {
            self.find_free_entry_run_in_cluster_chain(dir.cluster, count)
                .await
        }
    }

    /// Find `count` consecutive free entries in a fixed root directory.
    ///
    /// Returns DirectoryFull if no such run exists.
    async fn find_free_entry_run_in_fixed_root(
        &self,
        root_start: usize,
        root_size: usize,
        count: usize,
    ) -> Result<DirectoryEntryRun> {
        let mut data = self.data.lock();
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let max_entries = root_size / entry_size;
        let mut run = DirectoryEntryRun::new();
        let mut end_seen = false;

        for i in 0..max_entries {
            let offset = i * entry_size;
            let free = if end_seen {
                true
            } else {
                data.seek(SeekFrom::Start((root_start + offset) as u64))
                    .await?;
                let raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
                let first_byte = unsafe { raw_entry.bytes[0] };
                if first_byte == 0x00 {
                    end_seen = true;
                    true
                } else {
                    first_byte == 0xE5
                }
            };

            if free {
                run.push(DirectoryEntryPosition {
                    cluster: Cluster(0),
                    offset,
                });
                if run.len == count {
                    return Ok(run);
                }
            } else {
                run.clear();
            }
        }

        Err(FatError::DirectoryFull)
    }

    /// Find `count` consecutive free entries starting at the given cluster
    /// chain. Allocates a new cluster (extending the chain) when the existing
    /// space is exhausted. Free runs continue across cluster boundaries.
    async fn find_free_entry_run_in_cluster_chain(
        &self,
        dir_cluster: Cluster<usize>,
        count: usize,
    ) -> Result<DirectoryEntryRun> {
        let cluster_size = self.info.cluster_size;
        let entry_size = core::mem::size_of::<RawDirectoryEntry>();
        let entries_per_cluster = cluster_size / entry_size;
        let mut current_cluster = dir_cluster;
        let mut run = DirectoryEntryRun::new();
        let mut end_seen = false;
        // Bound the chain walk so a corrupt directory chain cannot loop
        // forever. Anything past `max_cluster` clusters has to revisit one.
        let chain_limit = self.fat.max_cluster();
        let mut steps: u32 = 0;

        loop {
            steps = steps.saturating_add(1);
            if steps > chain_limit {
                return Err(FatError::ClusterLoop {
                    cluster: current_cluster.0 as u32,
                });
            }
            {
                let mut data = self.data.lock();
                for i in 0..entries_per_cluster {
                    let offset = i * entry_size;
                    let free = if end_seen {
                        true
                    } else {
                        let seek_pos =
                            current_cluster.to_bytes(self.info.data_start, cluster_size) + offset;
                        data.seek(SeekFrom::Start(seek_pos as u64)).await?;
                        let raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
                        let first_byte = unsafe { raw_entry.bytes[0] };
                        if first_byte == 0x00 {
                            end_seen = true;
                            true
                        } else {
                            first_byte == 0xE5
                        }
                    };

                    if free {
                        run.push(DirectoryEntryPosition {
                            cluster: current_cluster,
                            offset,
                        });
                        if run.len == count {
                            return Ok(run);
                        }
                    } else {
                        run.clear();
                    }
                }
            }

            // Try to get next cluster (cache-routed).
            let next = self.next_cluster_routed(current_cluster.0).await?;
            match next {
                Some(cluster) => {
                    current_cluster = Cluster(cluster as usize);
                }
                None => {
                    // No more clusters: allocate a fresh one and link it in.
                    let hint = current_cluster.0 as u32 + 1;
                    let new_cluster = self.allocate_cluster_routed(hint).await?;
                    let new_cluster_pos = Cluster(new_cluster as usize)
                        .to_bytes(self.info.data_start, cluster_size);
                    let zero_result = {
                        let mut data = self.data.lock();
                        data.seek(SeekFrom::Start(new_cluster_pos as u64)).await?;
                        let zeros = alloc::vec![0u8; cluster_size];
                        data.write_all(&zeros).await
                    };
                    if let Err(error) = zero_result {
                        let _ = self.free_chain_routed(new_cluster).await;
                        return Err(error.into());
                    }
                    if let Err(error) = self
                        .write_clus_routed(current_cluster.0, new_cluster)
                        .await
                    {
                        let _ = self.free_chain_routed(new_cluster).await;
                        return Err(error);
                    }

                    self.decrement_free_count();
                    self.update_next_free_hint(new_cluster);
                    current_cluster = Cluster(new_cluster as usize);
                    end_seen = true;
                }
            }
        }
    }

    /// Write a raw directory entry at the specified location.
    ///
    /// For fixed root directory entries (cluster == 0), uses the fixed root offset.
    async fn write_raw_entry(
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

        data.seek(SeekFrom::Start(seek_pos as u64)).await?;
        data.write_all(bytemuck::bytes_of(entry)).await?;
        Ok(())
    }

    /// Write a raw `RawDirectoryEntry` (which may carry an LFN payload via
    /// the union variant). Identical seek logic to `write_raw_entry`; the
    /// split exists only because the short-entry caller already passes a
    /// `RawFileEntry`.
    #[cfg(feature = "lfn")]
    async fn write_raw_directory_entry(
        &self,
        cluster: Cluster<usize>,
        offset: usize,
        entry: &RawDirectoryEntry,
        fixed_root: Option<(usize, usize)>,
    ) -> Result<()> {
        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;
        let seek_pos = if cluster.0 == 0 {
            let (root_start, _) = fixed_root.expect("Fixed root info required for cluster 0");
            root_start + offset
        } else {
            cluster.to_bytes(self.info.data_start, cluster_size) + offset
        };
        data.seek(SeekFrom::Start(seek_pos as u64)).await?;
        // Safety: the union has a `bytes` variant guaranteed to be 32 bytes,
        // and the caller has already populated the entry through a typed
        // write. `bytemuck::bytes_of` is safe because RawDirectoryEntry is
        // Pod (NoUninit + AnyBitPattern declared in raw.rs).
        data.write_all(bytemuck::bytes_of(entry)).await?;
        Ok(())
    }

    /// Create a new file in the given directory.
    ///
    /// Returns the FileEntry for the newly created file.
    pub async fn create_file(&self, parent: &FatDir<'_, DATA>, name: &str) -> Result<FileEntry> {
        // Check if entry already exists
        if parent.find(name).await?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Generate short filename (suffix=0 means no ~N suffix)
        let short_name = ShortFileName::from_long_name_with(name, 0, self.oem_converter())
            .map_err(|_| FatError::InvalidFilename)?;

        // A name that fits 8.3 apart from per-part case is stored as a single
        // short entry with the NT case byte set; otherwise it needs LFN entries.
        // When the lfn feature is off, we never emit LFN.
        let case_bits = short_name_case_bits(name);
        #[cfg(feature = "lfn")]
        let mut lfn_buf: [RawDirectoryEntry; MAX_LFN_ENTRIES] = unsafe { core::mem::zeroed() };
        #[cfg(feature = "lfn")]
        let (lfn_count, nt_res) = match case_bits {
            Some(bits) => (0usize, bits),
            None => (
                build_lfn_entries(name, short_name.lfn_checksum(), &mut lfn_buf)
                    .ok_or(FatError::InvalidFilename)?,
                0u8,
            ),
        };
        #[cfg(not(feature = "lfn"))]
        let (lfn_count, nt_res) = (0usize, case_bits.unwrap_or(0));

        // Find a free run sized for the LFN preamble + the short entry.
        let total_slots = lfn_count + 1;
        let run = self.find_free_entry_run_in_dir(parent, total_slots).await?;

        // Write LFN entries first (in disk order), then the short entry.
        let now = self.time_provider().now();
        let (date, time, time_tenth) = now.to_raw();

        let mut raw_name = short_name.to_raw_bytes();
        kanji_short_name_fixup(&mut raw_name);

        let entry = RawFileEntry {
            name: raw_name,
            attributes: DirEntryAttrFlags::ARCHIVE.bits(),
            reserved: nt_res,
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

        #[cfg(feature = "lfn")]
        for (i, lfn_entry) in lfn_buf.iter().enumerate().take(lfn_count) {
            let position = run.get(i);
            self.write_raw_directory_entry(
                position.cluster,
                position.offset,
                lfn_entry,
                parent.fixed_root,
            )
                .await?;
        }
        // Short entry sits at the end of the run.
        let short_position = run.last();
        self.write_raw_entry(
            short_position.cluster,
            short_position.offset,
            &entry,
            parent.fixed_root,
        )
        .await?;

        // Flush FSInfo so on-disk free_count matches in-memory state (FAT32).
        // find_free_entry_slot_in_dir may have extended the parent directory.
        self.write_fsinfo().await?;

        Ok(FileEntry {
            short_name,
            nt_case: crate::raw::NtCaseFlags::from_bits_truncate(nt_res),
            #[cfg(feature = "lfn")]
            long_name: if lfn_count > 0 {
                crate::file::LongFileName::from_str_utf16(name)
            } else {
                None
            },
            attr: DirEntryAttrFlags::ARCHIVE,
            size: 0,
            parent_dir_clus: parent.cluster,
            parent_clus: short_position.cluster,
            offset_within_cluster: short_position.offset,
            cluster: Cluster(0),
            created: now,
            last_access_date: now.date,
            modified: crate::time::FatDateTime::from_raw(now.date, now.time, 0),
        })
    }

    /// Create a new directory.
    ///
    /// Returns a FatDir handle for the newly created directory.
    pub async fn create_dir<'a>(
        &'a self,
        parent: &FatDir<'a, DATA>,
        name: &str,
    ) -> Result<FatDir<'a, DATA>> {
        // Check if entry already exists
        if parent.find(name).await?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Generate short filename (suffix=0 means no ~N suffix)
        let short_name = ShortFileName::from_long_name_with(name, 0, self.oem_converter())
            .map_err(|_| FatError::InvalidFilename)?;

        // Allocate a cluster for the directory contents (cache-routed).
        let new_cluster = self.allocate_cluster_routed(2).await?;

        // Update FSInfo tracking (FAT32 only)
        self.decrement_free_count();
        self.update_next_free_hint(new_cluster);

        // A name that fits 8.3 apart from per-part case is stored as a single
        // short entry with the NT case byte set; otherwise it needs LFN entries.
        // When the lfn feature is off, we never emit LFN.
        let case_bits = short_name_case_bits(name);
        #[cfg(feature = "lfn")]
        let mut lfn_buf: [RawDirectoryEntry; MAX_LFN_ENTRIES] = unsafe { core::mem::zeroed() };
        #[cfg(feature = "lfn")]
        let (lfn_count, nt_res) = match case_bits {
            Some(bits) => (0usize, bits),
            None => (
                build_lfn_entries(name, short_name.lfn_checksum(), &mut lfn_buf)
                    .ok_or(FatError::InvalidFilename)?,
                0u8,
            ),
        };
        #[cfg(not(feature = "lfn"))]
        let (lfn_count, nt_res) = (0usize, case_bits.unwrap_or(0));

        // Allocate `lfn_count + 1` consecutive slots.
        let total_slots = lfn_count + 1;
        let run = self.find_free_entry_run_in_dir(parent, total_slots).await?;

        // Create the directory entry in parent
        let now = self.time_provider().now();
        let (date, time, time_tenth) = now.to_raw();

        // For FAT12/16, only use the low 16 bits of the cluster number
        let (cluster_high, cluster_low) = match &self.fat {
            Fat::Fat12(_) | Fat::Fat16(_) => (0u16, new_cluster as u16),
            Fat::Fat32(_) => ((new_cluster >> 16) as u16, new_cluster as u16),
        };

        let mut raw_name = short_name.to_raw_bytes();
        kanji_short_name_fixup(&mut raw_name);

        let entry = RawFileEntry {
            name: raw_name,
            attributes: DirEntryAttrFlags::DIRECTORY.bits(),
            reserved: nt_res,
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

        #[cfg(feature = "lfn")]
        for (i, lfn_entry) in lfn_buf.iter().enumerate().take(lfn_count) {
            let position = run.get(i);
            self.write_raw_directory_entry(
                position.cluster,
                position.offset,
                lfn_entry,
                parent.fixed_root,
            )
                .await?;
        }
        let short_position = run.last();
        let (slot_cluster, slot_offset) = (short_position.cluster, short_position.offset);
        self.write_raw_entry(slot_cluster, slot_offset, &entry, parent.fixed_root).await?;

        // Initialize the new directory with . and .. entries
        {
            let mut data = self.data.lock();
            let cluster_size = data.cluster_size;
            let dir_pos =
                Cluster(new_cluster as usize).to_bytes(self.info.data_start, cluster_size);

            // Zero out the cluster first
            data.seek(SeekFrom::Start(dir_pos as u64)).await?;
            let zeros = alloc::vec![0u8; cluster_size];
            data.write_all(&zeros).await?;

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
            data.seek(SeekFrom::Start(dir_pos as u64)).await?;
            data.write_all(bytemuck::bytes_of(&dot_entry)).await?;

            // Write ".." entry (points to parent).
            // FAT12/16 root has cluster 0 already, so it stores 0.
            // FAT32 spec: when the parent is the FAT32 root, ".." must store
            // cluster 0 even though the root has a real cluster — fsck.fat
            // rejects images that use the actual root cluster here.
            let parent_cluster = parent.cluster.0 as u32;
            let dotdot_cluster = if self.is_fat32_root_cluster(parent_cluster) {
                0
            } else {
                parent_cluster
            };
            let (parent_high, parent_low) = match &self.fat {
                Fat::Fat12(_) | Fat::Fat16(_) => (0u16, dotdot_cluster as u16),
                Fat::Fat32(_) => ((dotdot_cluster >> 16) as u16, dotdot_cluster as u16),
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
            data.seek(SeekFrom::Start(dotdot_pos as u64)).await?;
            data.write_all(bytemuck::bytes_of(&dotdot_entry)).await?;
        }

        // Flush FSInfo so on-disk free_count matches in-memory state (FAT32).
        self.write_fsinfo().await?;

        Ok(FatDir {
            data: self,
            cluster: Cluster(new_cluster as usize),
            fixed_root: None, // Newly created directories are never fixed root
        })
    }

    /// Delete a file or empty directory.
    pub async fn delete(&self, entry: &FileEntry) -> Result<()> {
        // If it's a directory, check if it's empty (only . and ..)
        if entry.is_directory() {
            let dir = FatDir {
                data: self,
                cluster: entry.cluster(),
                fixed_root: None, // User-created directories are never fixed root
            };

            let mut count = 0;
            let mut iter = dir.entries();
            while let Some(item) = iter.next_entry().await {
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

        // Free the cluster chain if there is one (cache-routed).
        if entry.cluster().0 >= 2 {
            let freed_count = self.free_chain_routed(entry.cluster().0 as u32).await?;
            // Update FSInfo tracking (FAT32 only)
            self.increment_free_count(freed_count);
        }

        // Mark the directory entry as deleted, plus any LFN slots that
        // precede it. Without the LFN cleanup, a delete would leave
        // orphaned LFN slots on disk — fsck.fat flags those as "stray
        // long-name slots" and they'd confuse a fresh-mount lookup until
        // the slots are eventually overwritten.
        self.mark_entry_span_deleted(entry).await?;

        // Flush FSInfo so on-disk free_count matches in-memory state (FAT32).
        self.write_fsinfo().await?;

        Ok(())
    }

    /// Rename or move a file or directory.
    ///
    /// Creates a new directory entry with `new_name` in `dest_dir`, copying
    /// the cluster chain, size, and attributes from the source entry, then
    /// marks the old entry as deleted. Data is NOT copied — only the
    /// directory entry metadata changes.
    ///
    /// If moving a directory to a different parent, the `..` entry is updated
    /// to point to the new parent.
    pub async fn rename(
        &self,
        entry: &FileEntry,
        dest_dir: &FatDir<'_, DATA>,
        new_name: &str,
    ) -> Result<FileEntry> {
        // Check if destination already has this name
        if dest_dir.find(new_name).await?.is_some() {
            return Err(FatError::AlreadyExists);
        }

        // Generate short filename
        let short_name = ShortFileName::from_long_name_with(new_name, 0, self.oem_converter())
            .map_err(|_| FatError::InvalidFilename)?;

        // The new name is stored as a single short entry (with NT case bits)
        // when it fits 8.3 apart from case, otherwise via LFN entries. When the
        // lfn feature is off, we never emit LFN.
        let case_bits = short_name_case_bits(new_name);
        #[cfg(feature = "lfn")]
        let mut lfn_buf: [RawDirectoryEntry; MAX_LFN_ENTRIES] = unsafe { core::mem::zeroed() };
        #[cfg(feature = "lfn")]
        let (lfn_count, nt_res) = match case_bits {
            Some(bits) => (0usize, bits),
            None => (
                build_lfn_entries(new_name, short_name.lfn_checksum(), &mut lfn_buf)
                    .ok_or(FatError::InvalidFilename)?,
                0u8,
            ),
        };
        #[cfg(not(feature = "lfn"))]
        let (lfn_count, nt_res) = (0usize, case_bits.unwrap_or(0));

        // Find a contiguous run sized for LFN entries plus the short entry.
        let total_slots = lfn_count + 1;
        let run = self
            .find_free_entry_run_in_dir(dest_dir, total_slots)
            .await?;
        #[cfg(feature = "lfn")]
        for (i, lfn_entry) in lfn_buf.iter().enumerate().take(lfn_count) {
            let position = run.get(i);
            self.write_raw_directory_entry(
                position.cluster,
                position.offset,
                lfn_entry,
                dest_dir.fixed_root,
            )
            .await?;
        }
        let short_position = run.last();
        let slot_cluster = short_position.cluster;
        let slot_offset = short_position.offset;

        // Read the original raw entry to preserve all fields
        let original_raw = {
            let mut data = self.data.lock();
            let cluster_size = data.cluster_size;

            let entry_pos = if entry.parent_clus.0 == 0 {
                let (root_start, _) = self
                    .fixed_root_dir_info()
                    .expect("Fixed root info required for cluster 0");
                root_start + entry.offset_within_cluster
            } else {
                entry
                    .parent_clus
                    .to_bytes(self.info.data_start, cluster_size)
                    + entry.offset_within_cluster
            };

            data.seek(SeekFrom::Start(entry_pos as u64)).await?;
            data.read_struct::<RawDirectoryEntry>().await?
        };

        // Build the new entry with the new name but same cluster/size/attributes
        let original_file = unsafe { &original_raw.file };
        let mut raw_name = short_name.to_raw_bytes();
        kanji_short_name_fixup(&mut raw_name);

        let now = self.time_provider().now();
        let new_entry = RawFileEntry {
            name: raw_name,
            // Case bits follow the new name, not the original entry's.
            reserved: nt_res,
            attributes: original_file.attributes,
            creation_time_tenth: original_file.creation_time_tenth,
            creation_time: original_file.creation_time,
            creation_date: original_file.creation_date,
            last_access_date: now.date.to_le_bytes(),
            first_cluster_high: original_file.first_cluster_high,
            last_write_time: now.time.to_le_bytes(),
            last_write_date: now.date.to_le_bytes(),
            first_cluster_low: original_file.first_cluster_low,
            size: original_file.size,
        };

        // Write the new entry
        self.write_raw_entry(slot_cluster, slot_offset, &new_entry, dest_dir.fixed_root)
            .await?;

        // If moving a directory to a different parent, update the ".." entry
        if entry.is_directory()
            && entry.cluster().0 >= 2
            && dest_dir.cluster != entry.parent_dir_clus
        {
            let mut data = self.data.lock();
            let cluster_size = data.cluster_size;
            let dir_data_start =
                entry.cluster().to_bytes(self.info.data_start, cluster_size);
            // ".." is the second entry (32 bytes after ".")
            let dotdot_pos = dir_data_start + core::mem::size_of::<RawDirectoryEntry>();
            data.seek(SeekFrom::Start(dotdot_pos as u64)).await?;
            let mut dotdot = data.read_struct::<RawDirectoryEntry>().await?;
            let dotdot_file = unsafe { &mut dotdot.file };

            // FAT32 spec: when the new parent is the FAT32 root, ".." stores
            // cluster 0 even though the root has a real cluster.
            let parent_cluster = dest_dir.cluster.0 as u32;
            let dotdot_cluster = if self.is_fat32_root_cluster(parent_cluster) {
                0
            } else {
                parent_cluster
            };
            let (parent_high, parent_low) = match &self.fat {
                Fat::Fat12(_) | Fat::Fat16(_) => (0u16, dotdot_cluster as u16),
                Fat::Fat32(_) => ((dotdot_cluster >> 16) as u16, dotdot_cluster as u16),
            };
            dotdot_file.first_cluster_high =
                hadris_common::types::number::U16::<LittleEndian>::new(parent_high);
            dotdot_file.first_cluster_low =
                hadris_common::types::number::U16::<LittleEndian>::new(parent_low);

            data.seek(SeekFrom::Start(dotdot_pos as u64)).await?;
            data.write_all(bytemuck::bytes_of(&dotdot)).await?;
        }

        // Mark the old entry deleted, including any preceding LFN slots so
        // we don't leave orphaned long-name entries behind.
        self.mark_entry_span_deleted(entry).await?;

        // Flush FSInfo so on-disk free_count matches in-memory state (FAT32).
        // find_free_entry_slot_in_dir on dest_dir may have extended it.
        self.write_fsinfo().await?;

        Ok(FileEntry {
            short_name,
            nt_case: crate::raw::NtCaseFlags::from_bits_truncate(nt_res),
            #[cfg(feature = "lfn")]
            long_name: if lfn_count > 0 {
                crate::file::LongFileName::from_str_utf16(new_name)
            } else {
                None
            },
            attr: DirEntryAttrFlags::from_bits_retain(original_file.attributes),
            size: original_file.size.get() as usize,
            parent_dir_clus: dest_dir.cluster,
            parent_clus: slot_cluster,
            offset_within_cluster: slot_offset,
            cluster: entry.cluster(),
            // Preserve original creation time; bump access/modified to "now".
            created: crate::time::FatDateTime::from_raw(
                u16::from_le_bytes(original_file.creation_date),
                u16::from_le_bytes(original_file.creation_time),
                original_file.creation_time_tenth,
            ),
            last_access_date: now.date,
            modified: crate::time::FatDateTime::from_raw(now.date, now.time, 0),
        })
    }

    /// Update a directory entry's size and first cluster fields.
    ///
    /// This is used by truncate and other operations that need to modify these fields.
    async fn update_entry_size_and_cluster(
        &self,
        entry: &FileEntry,
        new_size: usize,
        first_cluster: Cluster<usize>,
        fixed_root: Option<(usize, usize)>,
    ) -> Result<()> {
        use super::fat_table::Fat;

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
        data.seek(SeekFrom::Start(entry_pos as u64)).await?;

        let mut raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
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
        let now = self.time_provider().now();
        file_entry.last_write_date = now.date.to_le_bytes();
        file_entry.last_write_time = now.time.to_le_bytes();
        file_entry.last_access_date = now.date.to_le_bytes();

        // Write back the entry
        data.seek(SeekFrom::Start(entry_pos as u64)).await?;
        data.write_all(bytemuck::bytes_of(&raw_entry)).await?;

        Ok(())
    }
}

/// Volume label modification (root directory entry).
#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFs<DATA> {
    /// Overwrite the volume label stored in the root-directory entry.
    ///
    /// Returns [`FatError::EntryNotFound`] if no label entry exists today —
    /// callers should format the volume with a label, or extend the API
    /// later to allocate a new entry. The 11-byte name is written verbatim
    /// (FAT spec: space-padded, conventionally uppercase ASCII).
    ///
    /// This does **not** update the BPB volume label; reformatting is the
    /// only way to change that one without rewriting the boot sector.
    pub async fn set_root_label(&self, name: &[u8; 11]) -> Result<()> {
        let (pos, raw) = self
            .find_root_label_entry()
            .await?
            .ok_or(FatError::EntryNotFound)?;
        let mut updated = raw;
        // Writing to a union field of `Copy` type without `Drop` is safe in
        // modern Rust — the existing memory is overwritten verbatim.
        updated.file.name = *name;

        let mut data = self.data.lock();
        data.seek(SeekFrom::Start(pos as u64)).await?;
        data.write_all(bytemuck::bytes_of(&updated)).await?;
        data.flush().await?;
        Ok(())
    }
}

/// File attribute modification
#[cfg(feature = "write")]
impl<DATA: Read + Write + Seek> FatFs<DATA> {
    /// Set the attributes of a file or directory entry.
    ///
    /// Only the user-mutable bits (`READ_ONLY`, `HIDDEN`, `SYSTEM`, `ARCHIVE`)
    /// may be changed in place. Attempting to flip `DIRECTORY` or `VOLUME_ID`
    /// returns [`FatError::InvalidAttributeChange`] — those bits identify the
    /// kind of entry on disk and changing them would orphan a cluster chain
    /// or break the root volume label.
    pub async fn set_attributes(
        &self,
        entry: &FileEntry,
        attrs: DirEntryAttrFlags,
    ) -> Result<()> {
        // Reject flips on the immutable bits before touching disk.
        let current = entry.attributes();
        let immutable = DirEntryAttrFlags::DIRECTORY | DirEntryAttrFlags::VOLUME_ID;
        let changed = (current ^ attrs) & immutable;
        if changed.contains(DirEntryAttrFlags::DIRECTORY) {
            return Err(FatError::InvalidAttributeChange { bit: "DIRECTORY" });
        }
        if changed.contains(DirEntryAttrFlags::VOLUME_ID) {
            return Err(FatError::InvalidAttributeChange { bit: "VOLUME_ID" });
        }

        let mut data = self.data.lock();
        let cluster_size = data.cluster_size;

        let entry_pos = if entry.parent_clus.0 == 0 {
            let (root_start, _) = self
                .fixed_root_dir_info()
                .expect("Fixed root info required for cluster 0");
            root_start + entry.offset_within_cluster
        } else {
            entry
                .parent_clus
                .to_bytes(self.info.data_start, cluster_size)
                + entry.offset_within_cluster
        };

        // Read the current directory entry
        data.seek(SeekFrom::Start(entry_pos as u64)).await?;
        let mut raw_entry = data.read_struct::<RawDirectoryEntry>().await?;
        let file_entry = unsafe { &mut raw_entry.file };

        // Update attributes
        file_entry.attributes = attrs.bits();

        // Write back the entry
        data.seek(SeekFrom::Start(entry_pos as u64)).await?;
        data.write_all(bytemuck::bytes_of(&raw_entry)).await?;

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
    pub async fn sync(&self) -> Result<()> {
        self.write_fsinfo().await?;

        let mut data = self.data.lock();
        data.flush().await?;
        Ok(())
    }

    /// Write the FSInfo sector to disk (FAT32 only).
    ///
    /// This updates the free cluster count and next free cluster hint in the
    /// FSInfo sector. For FAT12/16 filesystems, this is a no-op.
    async fn write_fsinfo(&self) -> Result<()> {
        use super::fs::FatFsExt;
        use crate::raw::RawFsInfo;

        let ext = match &self.ext {
            FatFsExt::Fat32(ext) => ext,
            _ => return Ok(()), // No FSInfo for FAT12/16
        };

        let mut data = self.data.lock();

        // Seek to FSInfo sector
        data.seek_sector(ext.fs_info_sec).await?;

        // Read current FSInfo to preserve other fields
        let mut fs_info = data.read_struct::<RawFsInfo>().await?;

        // Update the mutable fields
        fs_info.free_count =
            hadris_common::types::number::U32::<LittleEndian>::new(ext.free_count.get());
        fs_info.next_free =
            hadris_common::types::number::U32::<LittleEndian>::new(ext.next_free.get().0);

        // Write back
        data.seek_sector(ext.fs_info_sec).await?;
        data.write_all(bytemuck::bytes_of(&fs_info)).await?;

        Ok(())
    }

    /// Decrement the free cluster count (called after cluster allocation).
    ///
    /// This only affects FAT32 filesystems.
    pub(crate) fn decrement_free_count(&self) {
        use super::fs::FatFsExt;

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
        use super::fs::FatFsExt;

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
        use super::fs::FatFsExt;

        if let FatFsExt::Fat32(ext) = &self.ext {
            // Set hint to the cluster after the one just allocated
            ext.next_free.set(Cluster(cluster.saturating_add(1)));
        }
    }

    /// Get the current free cluster count (FAT32 only).
    ///
    /// Returns `None` for FAT12/16 filesystems or if the count is unknown (0xFFFFFFFF).
    pub fn free_cluster_count(&self) -> Option<u32> {
        use super::fs::FatFsExt;

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
        use super::fs::FatFsExt;

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

/// Miri-targeted unit tests for `build_lfn_entries`. These exercise the
/// `unsafe { lfn: ... }` union writes inside the staging buffer and the
/// 0xFFFF padding writes that previously OOB'd at the spec cap (255 UTF-16
/// units). Pure functions, no I/O — so miri's no-syscall sandbox runs them
/// at full speed.
///
/// Wired into CI via `.github/workflows/rust.yml` (the `miri` job).
#[cfg(all(test, feature = "write", feature = "lfn"))]
mod lfn_write_safety_tests {
    use super::{build_lfn_entries, MAX_LFN_ENTRIES};
    use crate::raw::{DirEntryAttrFlags, RawDirectoryEntry};

    fn fresh_out() -> [RawDirectoryEntry; MAX_LFN_ENTRIES] {
        // SAFETY: zero-bytes is a valid bit pattern for every union variant
        // of RawDirectoryEntry — bytemuck::AnyBitPattern is impl'd on it.
        unsafe { core::mem::zeroed() }
    }

    /// All bytes of every written LFN slot must be readable through the
    /// `bytes` union arm without UB. Before this commit, an exactly-255
    /// UTF-16 name OOB'd the staging buffer; this test pins that fix.
    #[test]
    fn build_lfn_entries_at_spec_cap_255_units_does_not_oob() {
        let name: alloc::string::String = core::iter::repeat_n('a', 255).collect();
        let mut out = fresh_out();
        let n = build_lfn_entries(&name, 0, &mut out).expect("must accept 255 chars");
        assert_eq!(n, 20);
        for entry in out.iter().take(n) {
            // Touch every byte through the bytes union arm — miri flags
            // any out-of-bounds reads or invalid bit patterns.
            let bytes = unsafe { entry.bytes };
            assert_eq!(bytes.len(), 32);
        }
    }

    /// 256 UTF-16 units must surface as `None` (caller turns this into
    /// `InvalidFilename`) — silently truncating a filename is worse than
    /// refusing it.
    #[test]
    fn build_lfn_entries_overlong_returns_none() {
        let name: alloc::string::String = core::iter::repeat_n('a', 256).collect();
        let mut out = fresh_out();
        assert!(build_lfn_entries(&name, 0, &mut out).is_none());
    }

    /// Supplementary-plane chars (e.g. U+1F31F 🌟) need surrogate pairs in
    /// UTF-16 — 2 code units per char. The staging path writes both halves;
    /// miri verifies the writes stay within `u16_buf`.
    #[test]
    fn build_lfn_entries_supplementary_plane_writes_both_surrogates() {
        // 100 emoji × 2 UTF-16 units each = 200 units, fits the spec cap.
        let name: alloc::string::String = core::iter::repeat_n('\u{1F31F}', 100).collect();
        let mut out = fresh_out();
        let n = build_lfn_entries(&name, 0, &mut out).expect("100 emoji fits");
        // 200 units / 13 chars per entry = 16 (15.38 rounded up).
        assert_eq!(n, 16);
        // Sanity: the first slot's name1 starts with 0xD83C 0xDF1F or the
        // appropriate surrogate pair. Don't depend on which slot maps where —
        // just confirm that some slot contains valid surrogate halves.
        let mut saw_high = false;
        let mut saw_low = false;
        for entry in out.iter().take(n) {
            let lfn = unsafe { entry.lfn };
            for chunk in lfn.name1.chunks_exact(2) {
                let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
                if (0xD800..0xDC00).contains(&unit) {
                    saw_high = true;
                }
                if (0xDC00..0xE000).contains(&unit) {
                    saw_low = true;
                }
            }
        }
        assert!(saw_high && saw_low, "must encode both halves of the surrogate pair");
    }

    /// Exact fill (length is a multiple of 13): the spec says no
    /// terminator/filler is written. Verify the last LFN entry's bytes are
    /// all real chars, not 0xFFFF or 0x0000.
    #[test]
    fn build_lfn_entries_exact_13_unit_multiple_no_padding() {
        let name: alloc::string::String = core::iter::repeat_n('a', 13).collect();
        let mut out = fresh_out();
        let n = build_lfn_entries(&name, 0, &mut out).expect("13 chars fits");
        assert_eq!(n, 1);

        let lfn = unsafe { out[0].lfn };
        // All 13 units must be 'a' (0x0061). Walk name1 (5), name2 (6), name3 (2).
        for chunk in lfn
            .name1
            .chunks_exact(2)
            .chain(lfn.name2.chunks_exact(2))
            .chain(lfn.name3.chunks_exact(2))
        {
            let unit = u16::from_le_bytes([chunk[0], chunk[1]]);
            assert_eq!(unit, b'a' as u16, "exact-fill must contain only real chars");
        }
        // Sequence number has the LAST_ENTRY_MASK on the highest seq.
        assert_eq!(lfn.sequence_number, 0x41); // seq 1 + 0x40
        assert_eq!(lfn.attributes, DirEntryAttrFlags::LONG_NAME.bits());
    }

    /// The first entry on disk (out[0]) carries the highest sequence number
    /// with `LAST_ENTRY_MASK` set. This invariant is what readers rely on
    /// to find the start of an LFN run.
    #[test]
    fn build_lfn_entries_first_slot_has_last_entry_mask() {
        let name = "longishname.tx"; // 14 chars, 2 LFN entries
        let mut out = fresh_out();
        let n = build_lfn_entries(name, 0xAB, &mut out).expect("ok");
        assert_eq!(n, 2);
        let first = unsafe { out[0].lfn };
        assert_eq!(first.sequence_number, 0x42); // seq 2 | 0x40
        assert_eq!(first.checksum, 0xAB);
        let second = unsafe { out[1].lfn };
        assert_eq!(second.sequence_number, 0x01); // no mask
        assert_eq!(second.checksum, 0xAB);
    }
}

} // end io_transform!

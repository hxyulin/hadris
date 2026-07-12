//! File and directory layout types for metadata-only writing.
//!
//! These types require the `alloc` feature for heap allocation.

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::extent::{Extent, FileType, Timestamps};

/// File layout with pre-calculated extent (for metadata-only writing).
///
/// This structure describes where a file's data is located on disk
/// without containing the actual data. Used for operations that only
/// need to write/update filesystem metadata.
#[derive(Debug, Clone)]
pub struct FileLayout {
    /// File name (without path).
    pub name: String,
    /// Location and size of file data on disk.
    pub extent: Extent,
    /// Type of the file entry.
    pub file_type: FileType,
    /// File timestamps.
    pub timestamps: Timestamps,
    /// Filesystem-specific attribute flags.
    pub attributes: u32,
    /// Optional symlink target (only valid if file_type is Symlink).
    pub symlink_target: Option<String>,
}

impl FileLayout {
    /// Creates a new file layout.
    pub fn new(name: impl Into<String>, extent: Extent) -> Self {
        Self {
            name: name.into(),
            extent,
            file_type: FileType::RegularFile,
            timestamps: Timestamps::default(),
            attributes: 0,
            symlink_target: None,
        }
    }

    /// Sets the file type.
    pub fn with_type(mut self, file_type: FileType) -> Self {
        self.file_type = file_type;
        self
    }

    /// Sets the timestamps.
    pub fn with_timestamps(mut self, timestamps: Timestamps) -> Self {
        self.timestamps = timestamps;
        self
    }

    /// Sets the attributes.
    pub fn with_attributes(mut self, attributes: u32) -> Self {
        self.attributes = attributes;
        self
    }

    /// Sets the symlink target (only meaningful for symlinks).
    pub fn with_symlink_target(mut self, target: impl Into<String>) -> Self {
        self.symlink_target = Some(target.into());
        self
    }

    /// Returns the file size in bytes.
    #[inline]
    pub fn size(&self) -> u64 {
        self.extent.length
    }
}

/// Directory layout (tree of files).
///
/// Represents a complete directory tree with pre-calculated extents
/// for all files. Used for metadata-only writing operations.
#[derive(Debug, Clone, Default)]
pub struct DirectoryLayout {
    /// Directory name (empty string for root).
    pub name: String,
    /// Files in this directory.
    pub files: Vec<FileLayout>,
    /// Subdirectories.
    pub subdirs: Vec<DirectoryLayout>,
    /// Directory timestamps.
    pub timestamps: Timestamps,
    /// Filesystem-specific attribute flags.
    pub attributes: u32,
    /// Extent for the directory entry itself (if applicable).
    pub extent: Option<Extent>,
}

impl DirectoryLayout {
    /// Creates a new empty directory layout.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            subdirs: Vec::new(),
            timestamps: Timestamps::default(),
            attributes: 0,
            extent: None,
        }
    }

    /// Creates a root directory layout (empty name).
    pub fn root() -> Self {
        Self::new("")
    }

    /// Adds a file to this directory.
    pub fn add_file(&mut self, file: FileLayout) {
        self.files.push(file);
    }

    /// Adds a subdirectory to this directory.
    pub fn add_subdir(&mut self, subdir: DirectoryLayout) {
        self.subdirs.push(subdir);
    }

    /// Sets the timestamps.
    pub fn with_timestamps(mut self, timestamps: Timestamps) -> Self {
        self.timestamps = timestamps;
        self
    }

    /// Sets the extent for this directory entry.
    pub fn with_extent(mut self, extent: Extent) -> Self {
        self.extent = Some(extent);
        self
    }

    /// Returns the total number of files in this directory (not recursive).
    #[inline]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Returns the total number of subdirectories in this directory (not recursive).
    #[inline]
    pub fn subdir_count(&self) -> usize {
        self.subdirs.len()
    }

    /// Returns the total number of entries (files + subdirs).
    #[inline]
    pub fn entry_count(&self) -> usize {
        self.files.len() + self.subdirs.len()
    }

    /// Returns an iterator over all files (recursive, depth-first).
    pub fn iter_files(&self) -> impl Iterator<Item = (&str, &FileLayout)> {
        FileIterator::new(self)
    }

    /// Finds a file by path (e.g., "docs/readme.txt").
    pub fn find_file(&self, path: &str) -> Option<&FileLayout> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.find_file_parts(&parts)
    }

    /// Finds a file by path parts.
    fn find_file_parts(&self, parts: &[&str]) -> Option<&FileLayout> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            // Looking for a file in this directory
            self.files.iter().find(|f| f.name == parts[0])
        } else {
            // Looking in a subdirectory
            self.subdirs
                .iter()
                .find(|d| d.name == parts[0])
                .and_then(|d| d.find_file_parts(&parts[1..]))
        }
    }

    /// Finds a mutable file reference by path.
    pub fn find_file_mut(&mut self, path: &str) -> Option<&mut FileLayout> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.find_file_parts_mut(&parts)
    }

    /// Finds a mutable file reference by path parts.
    fn find_file_parts_mut(&mut self, parts: &[&str]) -> Option<&mut FileLayout> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            self.files.iter_mut().find(|f| f.name == parts[0])
        } else {
            self.subdirs
                .iter_mut()
                .find(|d| d.name == parts[0])
                .and_then(|d| d.find_file_parts_mut(&parts[1..]))
        }
    }

    /// Finds or creates a subdirectory by path.
    pub fn get_or_create_dir(&mut self, path: &str) -> &mut DirectoryLayout {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.get_or_create_dir_parts(&parts)
    }

    /// Finds or creates a subdirectory by path parts.
    fn get_or_create_dir_parts(&mut self, parts: &[&str]) -> &mut DirectoryLayout {
        if parts.is_empty() {
            return self;
        }

        let name = parts[0];

        // Find or create the subdirectory
        let idx = self.subdirs.iter().position(|d| d.name == name);
        let idx = match idx {
            Some(i) => i,
            None => {
                self.subdirs.push(DirectoryLayout::new(name));
                self.subdirs.len() - 1
            }
        };

        self.subdirs[idx].get_or_create_dir_parts(&parts[1..])
    }

    /// Removes a file by path. Returns the removed file if found.
    pub fn remove_file(&mut self, path: &str) -> Option<FileLayout> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.remove_file_parts(&parts)
    }

    /// Removes a file by path parts.
    fn remove_file_parts(&mut self, parts: &[&str]) -> Option<FileLayout> {
        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            let idx = self.files.iter().position(|f| f.name == parts[0])?;
            Some(self.files.remove(idx))
        } else {
            self.subdirs
                .iter_mut()
                .find(|d| d.name == parts[0])
                .and_then(|d| d.remove_file_parts(&parts[1..]))
        }
    }
}

/// Iterator over all files in a directory tree (depth-first).
struct FileIterator<'a> {
    stack: Vec<(&'a str, &'a DirectoryLayout, usize, usize)>,
    path_prefix: String,
}

impl<'a> FileIterator<'a> {
    fn new(root: &'a DirectoryLayout) -> Self {
        Self {
            stack: vec![("", root, 0, 0)],
            path_prefix: String::new(),
        }
    }
}

impl<'a> Iterator for FileIterator<'a> {
    type Item = (&'a str, &'a FileLayout);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((name, dir, file_idx, subdir_idx)) = self.stack.pop() {
            // Update path prefix when entering a new directory
            if !name.is_empty() {
                if !self.path_prefix.is_empty() {
                    self.path_prefix.push('/');
                }
                self.path_prefix.push_str(name);
            }

            // Return files first
            if file_idx < dir.files.len() {
                // Push state for next file
                self.stack.push((name, dir, file_idx + 1, subdir_idx));
                let file = &dir.files[file_idx];
                // We can't easily return the full path here without allocation
                // so we return just the file name
                return Some((&file.name, file));
            }

            // Then recurse into subdirectories
            if subdir_idx < dir.subdirs.len() {
                // Push state for next subdir
                self.stack.push((name, dir, file_idx, subdir_idx + 1));
                let subdir = &dir.subdirs[subdir_idx];
                self.stack.push((&subdir.name, subdir, 0, 0));
                continue;
            }

            // Pop path segment when leaving directory
            if !name.is_empty() {
                if let Some(idx) = self.path_prefix.rfind('/') {
                    self.path_prefix.truncate(idx);
                } else {
                    self.path_prefix.clear();
                }
            }
        }
        None
    }
}

/// Tracks which sectors are allocated in an image.
///
/// Uses a bitmap to efficiently track sector usage, supporting
/// allocation and deallocation operations.
#[derive(Debug, Clone)]
pub struct AllocationMap {
    /// Bitmap of used sectors (1 = used, 0 = free).
    bitmap: Vec<u8>,
    /// Total sectors in image.
    total_sectors: u32,
    /// Next free sector hint for faster allocation.
    next_free: u32,
}

impl AllocationMap {
    /// Creates a new allocation map for the given number of sectors.
    pub fn new(total_sectors: u32) -> Self {
        let bitmap_size = (total_sectors as usize).div_ceil(8);
        Self {
            bitmap: alloc::vec![0u8; bitmap_size],
            total_sectors,
            next_free: 0,
        }
    }

    /// Creates an allocation map from a list of existing used extents.
    pub fn from_existing(used_extents: &[Extent], total_sectors: u32, sector_size: u32) -> Self {
        let mut map = Self::new(total_sectors);
        for extent in used_extents {
            map.mark_used(*extent, sector_size);
        }
        map
    }

    /// Allocates a contiguous region of the given size.
    ///
    /// Returns `None` if there isn't enough contiguous free space.
    pub fn allocate(&mut self, size_bytes: u64, sector_size: u32) -> Option<Extent> {
        if size_bytes == 0 {
            return Some(Extent::new(self.next_free, 0));
        }

        let sectors_needed = size_bytes.div_ceil(sector_size as u64) as u32;

        // Start searching from next_free hint
        let mut start = self.next_free;
        let mut consecutive = 0u32;
        let mut found_start = start;

        while start + consecutive < self.total_sectors {
            let current = start + consecutive;
            if self.is_free(current) {
                if consecutive == 0 {
                    found_start = current;
                }
                consecutive += 1;
                if consecutive >= sectors_needed {
                    // Found enough space
                    let extent = Extent::new(found_start, size_bytes);
                    self.mark_used(extent, sector_size);
                    return Some(extent);
                }
            } else {
                // Reset search
                consecutive = 0;
                start = current + 1;
                found_start = start;
            }
        }

        // Try from beginning if we started after 0
        if self.next_free > 0 {
            start = 0;
            consecutive = 0;
            found_start = 0;

            while start + consecutive < self.next_free {
                let current = start + consecutive;
                if self.is_free(current) {
                    if consecutive == 0 {
                        found_start = current;
                    }
                    consecutive += 1;
                    if consecutive >= sectors_needed {
                        let extent = Extent::new(found_start, size_bytes);
                        self.mark_used(extent, sector_size);
                        return Some(extent);
                    }
                } else {
                    consecutive = 0;
                    start = current + 1;
                    found_start = start;
                }
            }
        }

        None
    }

    /// Marks the given extent as used.
    pub fn mark_used(&mut self, extent: Extent, sector_size: u32) {
        let end = extent.end_sector(sector_size);
        for sector in extent.sector..end {
            self.set_bit(sector, true);
        }
        // Update next_free hint
        if extent.sector == self.next_free {
            self.next_free = end;
            // Skip any used sectors
            while self.next_free < self.total_sectors && !self.is_free(self.next_free) {
                self.next_free += 1;
            }
        }
    }

    /// Marks the given extent as free.
    pub fn mark_free(&mut self, extent: Extent, sector_size: u32) {
        let end = extent.end_sector(sector_size);
        for sector in extent.sector..end {
            self.set_bit(sector, false);
        }
        // Update next_free hint if this freed earlier sectors
        if extent.sector < self.next_free {
            self.next_free = extent.sector;
        }
    }

    /// Checks if a sector is free.
    #[inline]
    pub fn is_free(&self, sector: u32) -> bool {
        if sector >= self.total_sectors {
            return false;
        }
        let byte_idx = sector as usize / 8;
        let bit_idx = sector % 8;
        (self.bitmap[byte_idx] & (1 << bit_idx)) == 0
    }

    /// Checks if a sector is used.
    #[inline]
    pub fn is_used(&self, sector: u32) -> bool {
        !self.is_free(sector)
    }

    /// Returns the total number of sectors.
    #[inline]
    pub fn total_sectors(&self) -> u32 {
        self.total_sectors
    }

    /// Returns the number of free sectors.
    pub fn free_sectors(&self) -> u32 {
        let mut count = 0u32;
        for sector in 0..self.total_sectors {
            if self.is_free(sector) {
                count += 1;
            }
        }
        count
    }

    /// Returns the number of used sectors.
    #[inline]
    pub fn used_sectors(&self) -> u32 {
        self.total_sectors - self.free_sectors()
    }

    /// Sets or clears a bit in the bitmap.
    #[inline]
    fn set_bit(&mut self, sector: u32, used: bool) {
        if sector >= self.total_sectors {
            return;
        }
        let byte_idx = sector as usize / 8;
        let bit_idx = sector % 8;
        if used {
            self.bitmap[byte_idx] |= 1 << bit_idx;
        } else {
            self.bitmap[byte_idx] &= !(1 << bit_idx);
        }
    }

    /// Reserves the first N sectors (typically for metadata).
    pub fn reserve_initial(&mut self, sectors: u32, sector_size: u32) {
        let extent = Extent::new(0, sectors as u64 * sector_size as u64);
        self.mark_used(extent, sector_size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_layout() {
        let file = FileLayout::new("test.txt", Extent::new(100, 1024))
            .with_type(FileType::RegularFile)
            .with_attributes(0x20);

        assert_eq!(file.name, "test.txt");
        assert_eq!(file.size(), 1024);
        assert_eq!(file.extent.sector, 100);
    }

    #[test]
    fn test_directory_layout() {
        let mut root = DirectoryLayout::root();
        root.add_file(FileLayout::new("file1.txt", Extent::new(100, 1024)));

        let mut subdir = DirectoryLayout::new("docs");
        subdir.add_file(FileLayout::new("readme.md", Extent::new(200, 512)));
        root.add_subdir(subdir);

        assert_eq!(root.file_count(), 1);
        assert_eq!(root.subdir_count(), 1);

        let file = root.find_file("file1.txt");
        assert!(file.is_some());
        assert_eq!(file.unwrap().name, "file1.txt");

        let nested = root.find_file("docs/readme.md");
        assert!(nested.is_some());
        assert_eq!(nested.unwrap().name, "readme.md");
    }

    #[test]
    fn test_get_or_create_dir() {
        let mut root = DirectoryLayout::root();
        let dir = root.get_or_create_dir("docs/api/v1");

        assert_eq!(dir.name, "v1");
        assert_eq!(root.subdirs[0].name, "docs");
        assert_eq!(root.subdirs[0].subdirs[0].name, "api");
        assert_eq!(root.subdirs[0].subdirs[0].subdirs[0].name, "v1");
    }

    #[test]
    fn test_remove_file() {
        let mut root = DirectoryLayout::root();
        root.add_file(FileLayout::new("test.txt", Extent::new(100, 1024)));

        let removed = root.remove_file("test.txt");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "test.txt");
        assert_eq!(root.file_count(), 0);
    }

    #[test]
    fn test_allocation_map() {
        let mut map = AllocationMap::new(100);
        assert_eq!(map.total_sectors(), 100);
        assert_eq!(map.free_sectors(), 100);

        // Allocate 10 sectors (20480 bytes with 2048-byte sectors)
        let extent = map.allocate(20480, 2048).unwrap();
        assert_eq!(extent.sector, 0);
        assert_eq!(extent.sector_count(2048), 10);
        assert_eq!(map.free_sectors(), 90);

        // Allocate more
        let extent2 = map.allocate(4096, 2048).unwrap();
        assert_eq!(extent2.sector, 10);

        // Free the first allocation
        map.mark_free(extent, 2048);
        assert_eq!(map.free_sectors(), 98);

        // New allocation should reuse freed space
        let extent3 = map.allocate(2048, 2048).unwrap();
        assert_eq!(extent3.sector, 0);
    }

    #[test]
    fn test_allocation_map_reserve() {
        let mut map = AllocationMap::new(100);
        map.reserve_initial(16, 2048); // Reserve sectors 0-15

        let extent = map.allocate(2048, 2048).unwrap();
        assert_eq!(extent.sector, 16); // Should start after reserved area
    }
}

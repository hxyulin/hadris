//! Shared directory tree representation for hybrid ISO+UDF images
//!
//! This module provides a file tree structure that can be used by both
//! ISO 9660 and UDF filesystem writers. The key insight is that both
//! filesystems point to the same physical file data on disk.

use std::path::PathBuf;
use std::sync::Arc;

/// Represents where a file's data lives on disk
#[derive(Debug, Clone, Copy, Default)]
pub struct FileExtent {
    /// Starting sector (logical block address)
    pub sector: u32,
    /// Length of the file data in bytes
    pub length: u64,
}

impl core::fmt::Display for FileExtent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_empty() {
            write!(f, "empty")
        } else {
            write!(f, "sector {} ({} bytes)", self.sector, self.length)
        }
    }
}

impl FileExtent {
    /// Create a new file extent
    pub fn new(sector: u32, length: u64) -> Self {
        Self { sector, length }
    }

    /// Check if this extent is empty (zero-size file)
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Calculate the number of sectors needed for this extent
    pub fn sector_count(&self, sector_size: usize) -> u32 {
        if self.length == 0 {
            0
        } else {
            ((self.length + sector_size as u64 - 1) / sector_size as u64) as u32
        }
    }
}

/// Source of file content
pub enum FileData {
    /// In-memory buffer
    Buffer(Vec<u8>),
    /// File on disk to read from
    Path(PathBuf),
}

impl std::fmt::Debug for FileData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffer(b) => write!(f, "Buffer({} bytes)", b.len()),
            Self::Path(p) => write!(f, "Path({:?})", p),
        }
    }
}

impl core::fmt::Display for FileData {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Buffer(b) => write!(f, "buffer ({} bytes)", b.len()),
            Self::Path(p) => write!(f, "path ({})", p.display()),
        }
    }
}

impl FileData {
    /// Get the size of the file data
    pub fn size(&self) -> std::io::Result<u64> {
        match self {
            Self::Buffer(b) => Ok(b.len() as u64),
            Self::Path(p) => Ok(std::fs::metadata(p)?.len()),
        }
    }

    /// Read the file content into a buffer
    pub fn read_all(&self) -> std::io::Result<Vec<u8>> {
        match self {
            Self::Buffer(b) => Ok(b.clone()),
            Self::Path(p) => std::fs::read(p),
        }
    }
}

/// A file entry in the directory tree
#[derive(Debug)]
pub struct FileEntry {
    /// File name
    pub name: Arc<String>,
    /// Physical location on disk (filled during layout phase)
    pub extent: FileExtent,
    /// Source of file content
    pub data: FileData,
    /// Unique ID for this file (used by UDF)
    pub unique_id: u64,
}

impl core::fmt::Display for FileEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl FileEntry {
    /// Create a new file entry from in-memory data
    pub fn from_buffer(name: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            name: Arc::new(name.into()),
            extent: FileExtent::default(),
            data: FileData::Buffer(data),
            unique_id: 0,
        }
    }

    /// Create a new file entry from a filesystem path
    pub fn from_path(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: Arc::new(name.into()),
            extent: FileExtent::default(),
            data: FileData::Path(path),
            unique_id: 0,
        }
    }

    /// Get the file size
    pub fn size(&self) -> std::io::Result<u64> {
        self.data.size()
    }
}

/// A directory in the file tree
#[derive(Debug)]
pub struct Directory {
    /// Directory name (empty for root)
    pub name: Arc<String>,
    /// Files in this directory
    pub files: Vec<FileEntry>,
    /// Subdirectories
    pub subdirs: Vec<Directory>,
    /// Unique ID for this directory (used by UDF)
    pub unique_id: u64,
    /// ICB location for UDF (logical block within partition)
    pub udf_icb_location: u32,
    /// Directory extent for ISO (sector and size)
    pub iso_extent: FileExtent,
}

impl core::fmt::Display for Directory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = if self.name.is_empty() { "/" } else { &self.name };
        write!(f, "{} ({} files, {} subdirs)", name, self.files.len(), self.subdirs.len())
    }
}

impl Directory {
    /// Create a new empty directory
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: Arc::new(name.into()),
            files: Vec::new(),
            subdirs: Vec::new(),
            unique_id: 0,
            udf_icb_location: 0,
            iso_extent: FileExtent::default(),
        }
    }

    /// Create an empty root directory
    pub fn root() -> Self {
        Self::new("")
    }

    /// Add a file to this directory
    pub fn add_file(&mut self, file: FileEntry) {
        self.files.push(file);
    }

    /// Add a subdirectory
    pub fn add_subdir(&mut self, dir: Directory) {
        self.subdirs.push(dir);
    }

    /// Find a file by name in this directory (not recursive)
    pub fn find_file(&self, name: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.name.as_str() == name)
    }

    /// Find a file by name in this directory (not recursive, mutable)
    pub fn find_file_mut(&mut self, name: &str) -> Option<&mut FileEntry> {
        self.files.iter_mut().find(|f| f.name.as_str() == name)
    }

    /// Find a subdirectory by name
    pub fn find_subdir(&self, name: &str) -> Option<&Directory> {
        self.subdirs.iter().find(|d| d.name.as_str() == name)
    }

    /// Find a subdirectory by name (mutable)
    pub fn find_subdir_mut(&mut self, name: &str) -> Option<&mut Directory> {
        self.subdirs.iter_mut().find(|d| d.name.as_str() == name)
    }

    /// Get the total number of files (recursive)
    pub fn total_files(&self) -> usize {
        self.files.len() + self.subdirs.iter().map(|d| d.total_files()).sum::<usize>()
    }

    /// Get the total number of directories (recursive, including self)
    pub fn total_dirs(&self) -> usize {
        1 + self.subdirs.iter().map(|d| d.total_dirs()).sum::<usize>()
    }

    /// Iterate over all files recursively
    pub fn iter_files(&self) -> Vec<&FileEntry> {
        let mut result: Vec<&FileEntry> = self.files.iter().collect();
        for subdir in &self.subdirs {
            result.extend(subdir.iter_files());
        }
        result
    }

    /// Sort files and directories by name
    pub fn sort(&mut self) {
        self.files.sort_by(|a, b| a.name.cmp(&b.name));
        self.subdirs.sort_by(|a, b| a.name.cmp(&b.name));
        for subdir in &mut self.subdirs {
            subdir.sort();
        }
    }
}

/// The complete file tree for a CD/DVD image
#[derive(Debug)]
pub struct FileTree {
    /// Root directory
    pub root: Directory,
}

impl core::fmt::Display for FileTree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} files, {} directories", self.total_files(), self.total_dirs())
    }
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTree {
    /// Create a new empty file tree
    pub fn new() -> Self {
        Self {
            root: Directory::root(),
        }
    }

    /// Add a file to the root directory
    pub fn add_file(&mut self, file: FileEntry) {
        self.root.add_file(file);
    }

    /// Add a directory to the root
    pub fn add_dir(&mut self, dir: Directory) {
        self.root.add_subdir(dir);
    }

    /// Find a file by path (e.g., "dir/subdir/file.txt")
    pub fn find_file(&self, path: &str) -> Option<&FileEntry> {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.is_empty() {
            return None;
        }

        let mut current = &self.root;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // Last part is the file name
                return current.find_file(part);
            } else {
                // Navigate to subdirectory
                current = current.find_subdir(part)?;
            }
        }
        None
    }

    /// Get the total number of files
    pub fn total_files(&self) -> usize {
        self.root.total_files()
    }

    /// Get the total number of directories (including root)
    pub fn total_dirs(&self) -> usize {
        self.root.total_dirs()
    }

    /// Sort all files and directories by name
    pub fn sort(&mut self) {
        self.root.sort();
    }

    /// Create a file tree from a filesystem directory
    pub fn from_fs(path: &std::path::Path) -> std::io::Result<Self> {
        let mut tree = Self::new();
        tree.root = Self::read_dir_recursive(path)?;
        tree.root.name = Arc::new(String::new()); // Root has empty name
        Ok(tree)
    }

    fn read_dir_recursive(path: &std::path::Path) -> std::io::Result<Directory> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let mut dir = Directory::new(name);

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let entry_name = entry.file_name().to_string_lossy().to_string();

            if file_type.is_file() {
                dir.add_file(FileEntry::from_path(entry_name, entry.path()));
            } else if file_type.is_dir() {
                dir.add_subdir(Self::read_dir_recursive(&entry.path())?);
            }
            // Skip symlinks and other file types for now
        }

        // Sort for consistent ordering
        dir.sort();
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_extent() {
        let extent = FileExtent::new(100, 4096);
        assert_eq!(extent.sector, 100);
        assert_eq!(extent.length, 4096);
        assert_eq!(extent.sector_count(2048), 2);

        let empty = FileExtent::default();
        assert!(empty.is_empty());
        assert_eq!(empty.sector_count(2048), 0);
    }

    #[test]
    fn test_directory_tree() {
        let mut tree = FileTree::new();

        // Add a file to root
        tree.add_file(FileEntry::from_buffer("readme.txt", b"Hello".to_vec()));

        // Add a subdirectory with a file
        let mut subdir = Directory::new("docs");
        subdir.add_file(FileEntry::from_buffer(
            "guide.txt",
            b"Guide content".to_vec(),
        ));
        tree.add_dir(subdir);

        assert_eq!(tree.total_files(), 2);
        assert_eq!(tree.total_dirs(), 2);

        // Find files
        assert!(tree.find_file("readme.txt").is_some());
        assert!(tree.find_file("docs/guide.txt").is_some());
        assert!(tree.find_file("nonexistent.txt").is_none());
    }
}

use core::ops::{Index, IndexMut};

use crate::structures::FatStr;

bitflags::bitflags! {
    /// File Attributes
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileAttributes: u8 {
        const READ_ONLY = 0x01;
        const HIDDEN = 0x02;
        const SYSTEM = 0x04;
        const VOLUME_LABEL = 0x08;
        const DIRECTORY = 0x10;
        const ARCHIVE = 0x20;
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct FileEntry {
    /// DIR_Name
    ///
    /// "Short" filename limited to 11 characters (8.3 format)
    filename: (FatStr<8>, FatStr<3>),
    /// DIR_Attr
    ///
    /// see FileAttributes
    attributes: FileAttributes,
    reserved: u8,
    creation_time_tenths: u8,
    creation_time: u16,
    creation_date: u16,
    last_accessed_date: u16,
    first_cluster_hi: u16,
    modification_time: u16,
    modification_date: u16,
    first_cluster_lo: u16,
    size: u32,
}

impl FileEntry {
    pub fn new(
        filename: &str,
        extension: &str,
        attributes: FileAttributes,
        size: u32,
        cluster: u32,
    ) -> Self {
        assert!(filename.len() <= FatStr::<8>::MAX_LEN);
        assert!(extension.len() <= FatStr::<3>::MAX_LEN);
        let filename = FatStr::new_truncate(filename);
        let extension = FatStr::new_truncate(extension);
        Self {
            filename: (filename, extension),
            attributes,
            reserved: 0,
            creation_time_tenths: 0,
            creation_time: 0,
            creation_date: 0,
            last_accessed_date: 0,
            first_cluster_hi: (cluster >> 16) as u16,
            modification_time: 0,
            modification_date: 0,
            first_cluster_lo: cluster as u16,
            size,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn cluster(&self) -> u32 {
        (self.first_cluster_hi as u32) << 16 | self.first_cluster_lo as u32
    }

    pub fn write_size(&mut self, size: u32) {
        self.size = size;
    }
}

unsafe impl bytemuck::Zeroable for FileEntry {}
unsafe impl bytemuck::NoUninit for FileEntry {}
unsafe impl bytemuck::AnyBitPattern for FileEntry {}

#[repr(transparent)]
pub struct Directory {
    pub entries: [FileEntry],
}

impl Directory {
    pub fn from_bytes<'a>(bytes: &'a [u8]) -> &'a Directory {
        assert!(bytes.len() % 32 == 0);
        let entries = bytemuck::cast_slice::<u8, FileEntry>(bytes);
        // SAFETY: 'Directory' is repr(transparent) over '[DirectoryEntry]'
        // so the fat pointer is safe to cast to a thin pointer
        unsafe { &*(entries as *const [FileEntry] as *const Directory) }
    }

    pub fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> &'a mut Directory {
        assert!(bytes.len() % 32 == 0);
        let entries = bytemuck::cast_slice_mut::<u8, FileEntry>(bytes);
        // SAFETY: 'Directory' is repr(transparent) over '[DirectoryEntry]'
        // so the fat pointer is safe to cast to a thin pointer
        unsafe { &mut *(entries as *mut [FileEntry] as *mut Directory) }
    }
}

impl Directory {
    /// Writes a new entry to the directory, returns the index of the entry if it was written
    /// If the directory is full, returns None, the user is expected to allocate more space
    pub fn write_entry(&mut self, entry: FileEntry) -> Option<usize> {
        // According to the spec, the first 4 bytes are zero if the entry is unused
        let mut index = 0xFFFF_FFFF;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.filename.0.raw == [0; 8] {
                index = i;
                break;
            }
        }
        if index == 0xFFFF_FFFF {
            return None;
        }

        self.entries[index] = entry;
        Some(index)
    }

    pub fn find_by_name(&self, base: &FatStr<8>, extension: &FatStr<3>) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.filename.0 == *base && entry.filename.1 == *extension {
                return Some(i);
            }
        }
        None
    }
}

impl Index<usize> for Directory {
    type Output = FileEntry;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

impl IndexMut<usize> for Directory {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

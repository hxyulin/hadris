use core::{
    ops::{Index, IndexMut},
    u16,
};

use crate::structures::FatStr;

use super::{
    raw::directory::RawFileEntry,
    time::{FatTime, FatTimeHighP},
};

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

impl TryFrom<hadris_core::file::FileAttributes> for FileAttributes {
    type Error = &'static str;

    fn try_from(value: hadris_core::file::FileAttributes) -> Result<Self, Self::Error> {
        use hadris_core::file::FileAttributes as Attributes;
        let mut attributes = FileAttributes::empty();
        if value.contains(Attributes::READ_ONLY) {
            attributes.set(FileAttributes::READ_ONLY, true);
        }
        if value.contains(Attributes::HIDDEN) {
            attributes.set(FileAttributes::HIDDEN, true);
        }

        //Err("Unsupported file attribute")
        Ok(attributes)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FileEntryInfo {
    pub basename: FatStr<8>,
    pub extension: FatStr<3>,
    pub attributes: FileAttributes,
    pub creation_time: FatTimeHighP,
    pub modification_time: FatTime,
    pub cluster: u32,
    pub size: u32,
}

impl TryFrom<&RawFileEntry> for FileEntryInfo {
    type Error = &'static str;

    fn try_from(value: &RawFileEntry) -> Result<Self, Self::Error> {
        let attributes =
            FileAttributes::from_bits(value.attributes).ok_or("Unsupported file attribute")?;
        let basename = FatStr::<8>::from_slice_unchecked(&value.name[0..8]);
        let extension = FatStr::<3>::from_slice_unchecked(&value.name[8..11]);
        let creation_time = FatTimeHighP::new(
            value.creation_time_tenth,
            u16::from_le_bytes(value.creation_time),
            u16::from_le_bytes(value.creation_date),
        );
        let modification_time = FatTime::new(
            u16::from_le_bytes(value.last_write_time),
            u16::from_le_bytes(value.last_write_date),
        );
        // TODO: Access date
        Ok(Self {
            basename,
            extension,
            attributes,
            creation_time,
            modification_time,
            cluster: ((u16::from_le_bytes(value.first_cluster_high) as u32) << 16)
                | u16::from_le_bytes(value.first_cluster_low) as u32,
            size: u32::from_le_bytes(value.size),
        })
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FileEntry {
    data: RawFileEntry,
}

impl FileEntry {
    pub fn new(
        filename: &str,
        extension: &str,
        attributes: FileAttributes,
        size: u32,
        cluster: u32,
        time: FatTimeHighP,
    ) -> Self {
        assert!(filename.len() <= FatStr::<8>::MAX_LEN);
        assert!(extension.len() <= FatStr::<3>::MAX_LEN);
        let filename = FatStr::<8>::new_truncate(filename);
        let extension = FatStr::<3>::new_truncate(extension);
        let mut name = [b' '; 11];
        name[0..8].copy_from_slice(filename.as_slice());
        name[8..11].copy_from_slice(extension.as_slice());

        Self {
            data: RawFileEntry {
                name,
                attributes: attributes.bits(),
                reserved: 0,
                creation_time_tenth: time.tenths,
                creation_time: time.time.time.to_le_bytes(),
                creation_date: time.time.date.to_le_bytes(),
                last_write_time: time.time.time.to_le_bytes(),
                last_write_date: time.time.date.to_le_bytes(),
                last_access_date: time.time.date.to_le_bytes(),
                first_cluster_high: ((cluster >> 16) as u16).to_le_bytes(),
                first_cluster_low: (cluster as u16).to_le_bytes(),
                size: size.to_le_bytes(),
            },
        }
    }

    pub fn size(&self) -> u32 {
        u32::from_le_bytes(self.data.size)
    }

    pub fn cluster(&self) -> u32 {
        let high = u16::from_le_bytes(self.data.first_cluster_high) as u32;
        let low = u16::from_le_bytes(self.data.first_cluster_low) as u32;
        (high << 16) | low
    }

    pub fn write_cluster(&mut self, cluster: u32) {
        let high = (cluster >> 16) as u16;
        let low = (cluster & u16::MAX as u32) as u16;
        self.data.first_cluster_high = high.to_le_bytes();
        self.data.first_cluster_low = low.to_le_bytes();
    }

    pub fn write_access_time(&mut self, time: FatTime) {
        self.data.last_access_date = time.date.to_le_bytes();
    }

    pub fn write_modification_time(&mut self, time: FatTime) {
        self.data.last_write_date = time.date.to_le_bytes();
        self.data.last_write_time = time.time.to_le_bytes();
    }

    pub fn write_size(&mut self, size: u32) {
        self.data.size = size.to_le_bytes();
    }

    pub fn base_name(&self) -> FatStr<8> {
        FatStr::from_slice_unchecked(&self.data.name[0..8])
    }

    pub fn extension(&self) -> FatStr<3> {
        FatStr::from_slice_unchecked(&self.data.name[8..11])
    }

    pub fn info(&self) -> FileEntryInfo {
        FileEntryInfo::try_from(&self.data).unwrap()
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
            // TODO: We need to check for the deallocated state as well
            if entry.base_name().raw == [0; 8] {
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
            if entry.base_name() == *base && entry.extension() == *extension {
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

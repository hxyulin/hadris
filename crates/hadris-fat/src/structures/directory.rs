use hadris_core::disk::{DiskError, DiskReader, DiskWriter};

use crate::structures::FatStr;

use super::{
    fat::Fat32,
    raw::directory::{RawDirectoryEntry, RawFileEntry},
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
#[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::AnyBitPattern)]
pub struct FileEntry {
    data: RawFileEntry,
}

impl FileEntry {
    pub fn from_bytes(bytes: &[u8]) -> &FileEntry {
        bytemuck::from_bytes(bytes)
    }
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
        assert!(
            !attributes.contains(FileAttributes::DIRECTORY) || size == 0,
            "Size must be zero for directories"
        );
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

    pub fn attributes(&self) -> FileAttributes {
        FileAttributes::from_bits(self.data.attributes).unwrap()
    }
}

pub struct Directory {
    /// The offset of directory in bytes (precomputed)
    /// This is essentially the start of the data area
    root_directory_offset: usize,
    /// Size of each cluster in bytes
    cluster_size: usize,
}

#[cfg(feature = "read")]
impl Directory {
    pub fn new(root_directory_offset: usize, cluster_size: usize) -> Directory {
        Self {
            root_directory_offset,
            cluster_size,
        }
    }

    /// Finds a directory entry by name and extension.
    /// Returns the **index of the entry** in the directory if found.
    pub fn find_entry<R: DiskReader>(
        &self,
        reader: &mut R,
        fat: &mut Fat32,
        mut current_cluster: u32,
        name: FatStr<8>,
        extension: FatStr<3>,
    ) -> Result<Option<usize>, DiskError> {
        assert!(
            current_cluster >= 2,
            "Cluster number must be greater than 2"
        );

        let mut buffer = [0u8; 512];
        let mut index = 0;
        let entries_per_cluster = self.cluster_size / size_of::<RawDirectoryEntry>();

        loop {
            let cluster_offset =
                (current_cluster as usize - 2) * self.cluster_size + self.root_directory_offset;
            reader.read_bytes(cluster_offset, &mut buffer)?;

            for (entry_index, entry_bytes) in buffer
                .chunks_exact(size_of::<RawDirectoryEntry>())
                .enumerate()
            {
                if entry_bytes[0] == 0x00 {
                    return Ok(None);
                }

                let entry = FileEntry::from_bytes(entry_bytes);

                if entry.base_name() == name && entry.extension() == extension {
                    return Ok(Some(index * entries_per_cluster + entry_index));
                }
            }

            index += 1;
            current_cluster = fat.next_cluster_index(reader, current_cluster)?;
            if current_cluster < 2 || current_cluster >= 0x0FFFFFF8 {
                return Ok(None);
            }
        }
    }

    pub fn get_entry<R: DiskReader>(
        &self,
        reader: &mut R,
        cluster: u32,
        index: usize,
    ) -> FileEntry {
        let mut buffer = [0u8; 32];
        let cluster_offset =
            (cluster as usize - 2) * self.cluster_size + self.root_directory_offset;
        let offset = cluster_offset + size_of::<RawDirectoryEntry>() * index;
        reader.read_bytes(offset, &mut buffer).unwrap();
        bytemuck::cast(buffer)
    }
}

#[cfg(feature = "write")]
impl Directory {
    pub fn write_entry<W: DiskReader + DiskWriter>(
        &mut self,
        writer: &mut W,
        cluster: u32,
        entry: &FileEntry,
    ) -> Result<usize, DiskError> {
        assert!(cluster >= 2, "Cluster number must be greater than 2");

        let mut buffer = [0u8; 512];
        let index = 0;
        let entries_per_cluster = self.cluster_size / size_of::<RawDirectoryEntry>();

        let cluster_offset =
            (cluster as usize - 2) * self.cluster_size + self.root_directory_offset;
        writer.read_bytes(cluster_offset, &mut buffer)?;

        for (entry_index, entry_bytes) in buffer
            .chunks_exact_mut(size_of::<RawDirectoryEntry>())
            .enumerate()
        {
            if entry_bytes[0] == 0x00 || entry_bytes[0] == 0xE5 {
                entry_bytes.copy_from_slice(bytemuck::bytes_of(entry));
                writer.write_bytes(cluster_offset, &buffer)?;
                return Ok(index * entries_per_cluster + entry_index);
            }
        }
        // TODO: We should return an error, or at elast try to allocate a cluster
        panic!("Could not find free entry");
    }
}

#[cfg(all(test, feature = "std"))]
mod test {
    use super::*;

    #[test]
    fn test_find_entry_single_sector() {
        let mut directory = [0u8; 1024];

        // We need to create dummy fat
        directory[0..4].copy_from_slice(&0xFFFF_FFF8_u32.to_le_bytes());
        directory[4..8].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        // We just mark the root cluster as EOC
        directory[8..12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());

        let mut fat = Fat32::new(0, 512, 1, 512);
        let reader = Directory::new(512, 512);

        // One sector of fat and one sector of directory
        let entry = FileEntry::new(
            "test",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        directory[512..512 + size_of::<FileEntry>()].copy_from_slice(bytemuck::bytes_of(&entry));

        let entry = reader
            .find_entry(
                &mut directory.as_slice(),
                &mut fat,
                2,
                FatStr::new_truncate("test"),
                FatStr::new_truncate("txt"),
            )
            .unwrap();
        assert_eq!(entry, Some(0));

        let entry = reader
            .find_entry(
                &mut directory.as_slice(),
                &mut fat,
                2,
                FatStr::new_truncate("unknown"),
                FatStr::new_truncate("txt"),
            )
            .unwrap();
        assert_eq!(entry, None);
    }

    #[test]
    fn test_find_entry_multi_sector() {
        let mut directory = [0u8; 512 * 4];

        directory[0..4].copy_from_slice(&0xFFFF_FFF8_u32.to_le_bytes());
        directory[4..8].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        // We link to the next cluster
        directory[8..12].copy_from_slice(&3_u32.to_le_bytes());
        // Now we mark it as the last cluster
        directory[12..16].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());

        // We need to fill all the other entries because they can't be zeroes
        let entry = FileEntry::new(
            "dummy",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        for i in 0..960 / size_of::<FileEntry>() {
            directory[512 + i * size_of::<FileEntry>()
                ..512 + i * size_of::<FileEntry>() + size_of::<FileEntry>()]
                .copy_from_slice(bytemuck::bytes_of(&entry));
        }
        let entry = FileEntry::new(
            "test",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        directory[512 + 960..512 + 960 + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));
        let mut fat_reader = Fat32::new(0, 512, 1, 512);
        let reader = Directory::new(512, 512);
        let index = 960 / size_of::<FileEntry>();
        let entry = reader
            .find_entry(
                &mut directory.as_slice(),
                &mut fat_reader,
                2,
                FatStr::new_truncate("test"),
                FatStr::new_truncate("txt"),
            )
            .unwrap();
        assert_eq!(entry, Some(index));
    }

    #[test]
    fn test_find_entry_multi_sector_fragmented() {
        let mut directory = [0u8; 512 * 8];
        // So the fat will be first sector,
        // and the root directory will be the second sector
        // The we link the next cluster of the root directory to cluster 6 for non contiguous test
        directory[0..4].copy_from_slice(&0xFFFF_FFF8_u32.to_le_bytes());
        directory[4..8].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        directory[8..12].copy_from_slice(&6_u32.to_le_bytes());
        directory[24..28].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());

        // Now we fill the root cluster with dummy data
        let entry = FileEntry::new(
            "dummy",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        for i in 0..512 / size_of::<RawDirectoryEntry>() {
            let offset = i * size_of::<RawDirectoryEntry>() + 512;
            directory[offset..offset + size_of::<RawDirectoryEntry>()]
                .copy_from_slice(&bytemuck::bytes_of(&entry));
        }

        // Now we make it the first entry of cluster 6
        let entry = FileEntry::new(
            "test",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );

        // We start root cluster (2) at cluster 1, so we need  to subtract 1 from the cluster number
        let offset = (6 - 1) * 512;
        directory[offset..offset + size_of::<RawDirectoryEntry>()]
            .copy_from_slice(&bytemuck::bytes_of(&entry));

        let reader = Directory::new(512, 512);
        let mut fat_reader = Fat32::new(0, 512, 1, 512);
        let entry = reader
            .find_entry(
                &mut directory.as_slice(),
                &mut fat_reader,
                2,
                FatStr::new_truncate("test"),
                FatStr::new_truncate("txt"),
            )
            .unwrap();
        assert_eq!(Some(16), entry);
    }

    // TESTS: Maybe add tests for the last possible entry in a cluster, and maybe some with deleted
    // entries (0xE5 marker)

    #[test]
    fn test_create_directory() {
        let mut directory = [0u8; 512];
        let mut writer = Directory::new(0, 512);
        let entry = FileEntry::new(
            "test",
            "",
            FileAttributes::DIRECTORY,
            0,
            1,
            FatTimeHighP::default(),
        );
        let result = writer
            .write_entry(&mut directory.as_mut_slice(), 2, &entry)
            .unwrap();
        assert_eq!(result, 0);
    }
}

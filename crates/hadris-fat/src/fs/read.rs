use hadris_core::{ReadWriteError, Reader};

use crate::structures::{
    boot_sector::BootSectorFat32,
    directory::{DirectoryReader, FileAttributes, FileEntry},
    fat::Fat32Reader,
    FatStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemError {
    IOError(ReadWriteError),
    IsFile,
    IsDirectory,
    NotFound,
}

impl From<ReadWriteError> for FileSystemError {
    fn from(value: ReadWriteError) -> Self {
        Self::IOError(value)
    }
}

pub struct FileSystemRead {}

impl FileSystemRead {
    pub fn find_file_entry(
        &self,
        reader: &mut dyn Reader,
        name: &str,
    ) -> Result<Option<FileEntry>, FileSystemError> {
        let mut temp_buffer = [0u8; 512];
        reader.read_sector(0, &mut temp_buffer)?;
        let bpb = BootSectorFat32::create_from_bytes(temp_buffer);

        let bytes = name.as_bytes();
        let mut buf = FatStr::<8>::default();
        // We allow an optional leading slash
        let mut index = if bytes[0] == b'/' { 1 } else { 0 };

        let cluster_size = bpb.sectors_per_cluster() as usize * bpb.bytes_per_sector() as usize;
        let directory_offset = (bpb.reserved_sector_count() as usize
            + bpb.fat_count() as usize * bpb.sectors_per_fat() as usize)
            * bpb.bytes_per_sector() as usize;
        let mut current_cluster = bpb.root_directory_cluster();
        let mut fat_reader = Fat32Reader::new(
            bpb.reserved_sector_count() as usize * bpb.bytes_per_sector() as usize,
            bpb.sectors_per_fat() as usize * bpb.bytes_per_sector() as usize,
            bpb.fat_count() as usize,
        );

        loop {
            if let Some(idx) = bytes.iter().skip(index).position(|b| *b == b'/') {
                buf.clear();
                buf.copy_from_slice(bytes.get(index..index + idx).unwrap());
                index += idx + 1;
                let directory = DirectoryReader::new(directory_offset, cluster_size);
                let entry = directory
                    .find_entry(
                        reader,
                        &mut fat_reader,
                        current_cluster,
                        buf,
                        FatStr::default(),
                    )?
                    .ok_or(FileSystemError::NotFound)?;
                let entry = directory.get_entry(reader, current_cluster, entry);
                let remaining_bytes = &bytes[index..];
                if !entry.attributes().contains(FileAttributes::DIRECTORY) {
                    return Err(FileSystemError::IsFile);
                }
                if remaining_bytes.is_empty() {
                    // We just return the directory, in the case the user specified a path like
                    // this:
                    // /test/folder/
                    return Ok(Some(entry));
                }
                if entry.cluster() == 0 || entry.size() == 0 {
                    return Err(FileSystemError::NotFound);
                }
                current_cluster = entry.cluster();
            } else {
                let mut name = FatStr::<8>::default();
                let mut extension = FatStr::<3>::default();
                let dot_index = bytes.iter().skip(index).position(|b| *b == b'.');
                if let Some(dot_index) = dot_index {
                    // TODO: Dot as the last character is an invalid path
                    assert!(dot_index < bytes.len() - 1); // This should be an error
                    name.copy_from_slice(&bytes[index..index + dot_index]);
                    extension.copy_from_slice(&bytes[index + dot_index + 1..]);
                } else {
                    // TODO: Support LFN
                    assert!(bytes.len() - index < FatStr::<8>::MAX_LEN);
                    name.copy_from_slice(&bytes[index..]);
                };

                let directory = DirectoryReader::new(directory_offset, cluster_size);
                let entry = directory
                    .find_entry(reader, &mut fat_reader, current_cluster, name, extension)?
                    .ok_or(FileSystemError::NotFound)?;
                return Ok(Some(directory.get_entry(reader, current_cluster, entry)));
            }
        }
    }
    /// Read a file from the filesystem
    pub fn read_file_raw(
        &self,
        reader: &mut dyn Reader,
        cluster_start: u32,
        offset: u32,
        buffer: &mut [u8],
    ) -> Result<usize, ReadWriteError> {
        // We need to read BPB first for some important info
        let mut temp_buffer = [0u8; 512];
        reader.read_sector(0, &mut temp_buffer)?;
        let bpb = BootSectorFat32::create_from_bytes(temp_buffer);

        // TODO: For now we dont care about the file entry, we just read the fat clusters
        let fat_start = bpb.reserved_sector_count() as usize * bpb.bytes_per_sector() as usize;
        let fat_size = bpb.sectors_per_fat() as usize * bpb.bytes_per_sector() as usize;
        let fat_reader = Fat32Reader::new(fat_start, fat_size, bpb.fat_count() as usize);
        let cluster_size = bpb.sectors_per_cluster() as usize * bpb.bytes_per_sector() as usize;

        fat_reader.read_data(reader, cluster_size, cluster_start, offset as usize, buffer)
    }
}

#[cfg(test)]
mod tests {
    use crate::structures::{directory::FileAttributes, time::FatTimeHighP};

    use super::*;

    struct TestFs<'a> {
        fs: crate::FileSystem<'a>,
        ops: crate::structures::Fat32Ops,
    }

    impl<'a> TestFs<'a> {
        fn new(data: &'a mut [u8]) -> Self {
            let sectors = data.len() / 512;
            let ops = crate::structures::Fat32Ops::recommended_config_for(sectors as u32);
            let fs = crate::FileSystem::new_f32(ops.clone(), data);
            assert_eq!(ops.root_cluster, 2);
            Self { fs, ops }
        }

        fn fat_offset(&self) -> usize {
            self.ops.reserved_sector_count as usize * self.ops.bytes_per_sector as usize
        }

        fn root_directory_offset(&self) -> usize {
            (self.ops.reserved_sector_count as usize
                + self.ops.sectors_per_fat_32 as usize * self.ops.fat_count as usize)
                * self.ops.bytes_per_sector as usize
        }
    }

    #[test]
    fn test_find_root_file() {
        let path1 = "/test.txt";
        let path2 = "test.txt";

        let sectors = 1024;
        let mut data: Vec<u8> = Vec::with_capacity(1024 * 512);
        data.resize(sectors * 512, 0);
        let fs = TestFs::new(&mut data);

        let fat_offset = fs.fat_offset();
        let root_offset = fs.root_directory_offset();

        data[fat_offset + 8..fat_offset + 12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let entry = FileEntry::new(
            "test",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        data[root_offset..root_offset + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));

        let reader = FileSystemRead {};
        let entry = reader.find_file_entry(&mut data.as_slice(), path1).unwrap();
        assert_eq!(entry.unwrap().base_name().as_str(), "test    ");
        assert_eq!(entry.unwrap().extension().as_str(), "txt");

        let entry = reader.find_file_entry(&mut data.as_slice(), path2).unwrap();
        assert_eq!(entry.unwrap().base_name().as_str(), "test    ");
        assert_eq!(entry.unwrap().extension().as_str(), "txt");
    }

    #[test]
    fn test_find_root_directory() {
        // Find the test directory
        let path = "test/";

        let sectors = 1024;
        let mut data: Vec<u8> = Vec::with_capacity(1024 * 512);
        data.resize(sectors * 512, 0);
        let fs = TestFs::new(&mut data);

        let fat_offset = fs.fat_offset();
        let root_offset = fs.root_directory_offset();
        data[fat_offset + 8..fat_offset + 12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let entry = FileEntry::new(
            "test",
            "",
            FileAttributes::DIRECTORY,
            0,
            0,
            FatTimeHighP::default(),
        );
        let bytes: [u8; 32] = bytemuck::cast(entry);
        data[root_offset..root_offset + 32].copy_from_slice(&bytes);

        let reader = FileSystemRead {};
        let entry = reader
            .find_file_entry(&mut data.as_slice(), path)
            .unwrap()
            .unwrap();
        assert_eq!(entry.base_name().as_str(), "test    ");
    }

    #[test]
    fn test_find_nested_file() {
        let path1 = "dir/test.txt";
        let path2 = "/dir/test.txt";

        let sectors = 1024;
        let mut data: Vec<u8> = Vec::with_capacity(1024 * 512);
        data.resize(sectors * 512, 0);
        let fs = TestFs::new(&mut data);

        let fat_offset = fs.fat_offset();
        let root_offset = fs.root_directory_offset();

        data[fat_offset + 8..fat_offset + 12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let entry = FileEntry::new(
            "dir",
            "",
            FileAttributes::DIRECTORY,
            512,
            3,
            FatTimeHighP::default(),
        );
        data[root_offset..root_offset + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));

        data[fat_offset + 12..fat_offset + 16].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let directory_offset = root_offset + 512; // Its the next cluster
        let entry = FileEntry::new(
            "test",
            "txt",
            FileAttributes::empty(),
            0,
            0,
            FatTimeHighP::default(),
        );
        data[directory_offset..directory_offset + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));

        let reader = FileSystemRead {};
        let entry = reader
            .find_file_entry(&mut data.as_slice(), path1)
            .unwrap()
            .unwrap();
        assert_eq!(entry.base_name().as_str(), "test    ");
        assert_eq!(entry.extension().as_str(), "txt");

        let entry = reader
            .find_file_entry(&mut data.as_slice(), path2)
            .unwrap()
            .unwrap();
        assert_eq!(entry.base_name().as_str(), "test    ");
        assert_eq!(entry.extension().as_str(), "txt");
    }
    
    #[test]
    fn test_find_nested_directory() {
        let path1 = "test/test/";
        let path2 = "/test/test/";

        let sectors = 1024;
        let mut data: Vec<u8> = Vec::with_capacity(1024 * 512);
        data.resize(sectors * 512, 0);
        let fs = TestFs::new(&mut data);

        let fat_offset = fs.fat_offset();
        let root_offset = fs.root_directory_offset();
        data[fat_offset + 8..fat_offset + 12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let entry = FileEntry::new(
            "test",
            "",
            FileAttributes::DIRECTORY,
            512,
            3,
            FatTimeHighP::default(),
        );
        data[root_offset..root_offset + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));

        data[fat_offset + 12..fat_offset + 16].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        let directory_offset = root_offset + 512; // Its the next cluster
        let entry = FileEntry::new(
            "test",
            "",
            FileAttributes::DIRECTORY,
            0,
            0,
            FatTimeHighP::default(),
        );
        data[directory_offset..directory_offset + size_of::<FileEntry>()]
            .copy_from_slice(bytemuck::bytes_of(&entry));

        let reader = FileSystemRead {};
        let entry = reader
            .find_file_entry(&mut data.as_slice(), path1)
            .unwrap()
            .unwrap();
        assert_eq!(entry.base_name().as_str(), "test    ");
        assert_eq!(entry.extension().as_str(), "   ");

        let entry = reader
            .find_file_entry(&mut data.as_slice(), path2)
            .unwrap()
            .unwrap();
        assert_eq!(entry.base_name().as_str(), "test    ");
        assert_eq!(entry.extension().as_str(), "   ");
    }
}

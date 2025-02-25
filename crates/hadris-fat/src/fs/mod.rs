use bytemuck::Zeroable;
use hadris_core::{
    path::Path,
    str::{AsciiStr, FixedByteStr},
    ReadWriteError, Reader, Writer,
};

#[cfg(feature = "write")]
use crate::structures::Fat32Ops;
use crate::structures::{
    boot_sector::{BootSector, BootSectorFat32, BootSectorInfo, BootSectorInfoFat32},
    directory::{Directory, FileAttributes, FileEntry, FileEntryInfo},
    fat::Fat32,
    fs_info::{FsInfo, FsInfoInfo},
    raw::directory::RawDirectoryEntry,
    time::FatTimeHighP,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemError {
    IOError(ReadWriteError),
    NotFound,
}

impl From<ReadWriteError> for FileSystemError {
    fn from(value: ReadWriteError) -> Self {
        Self::IOError(value)
    }
}

impl core::fmt::Display for FileSystemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IOError(error) => write!(f, "IO error: {}", error),
            Self::NotFound => f.write_str("Not found"),
        }
    }
}

impl core::error::Error for FileSystemError {}

#[derive(Debug, Clone)]
pub struct FatFs32 {
    bs: BootSectorInfoFat32,
    fs_info: FsInfoInfo,
}

#[cfg(feature = "read")]
impl FatFs32 {
    pub fn read(reader: &mut dyn Reader) -> Result<Self, FileSystemError> {
        let mut temp_buffer = [0u8; 512];
        reader.read_sector(0, &mut temp_buffer)?;
        let bs = BootSectorFat32::from_bytes(&temp_buffer).info();
        reader.read_sector(bs.fs_info_sector as u32, &mut temp_buffer)?;
        let fs_info = FsInfo::from_bytes(&temp_buffer).info();

        Ok(Self { bs, fs_info })
    }

    pub fn list_dir<'a>(
        &'a self,
        reader: &'a mut dyn Reader,
        path: &Path,
    ) -> Result<DirectoryIter<'a>, FileSystemError> {
        let cluster = if path.is_root() {
            self.bs.root_cluster
        } else {
            unimplemented!("Non root directory not supported")
        };

        let fat_offset = self.bs.reserved_sector_count as usize * self.bs.bytes_per_sector as usize;
        let root_directory_offset = self.bs.sectors_per_fat as usize
            * self.bs.fat_count as usize
            * self.bs.bytes_per_sector as usize
            + fat_offset;

        DirectoryIter::new(
            reader,
            fat_offset,
            root_directory_offset,
            cluster,
            self.bs.bytes_per_sector as usize,
        )
    }
}

#[cfg(feature = "write")]
impl FatFs32 {
    pub fn create_fat32(writer: &mut dyn Writer, ops: Fat32Ops) -> Result<FatFs32, ReadWriteError> {
        let boot_sector = BootSector::create_fat32(
            ops.jmp_boot_code,
            ops.bytes_per_sector,
            ops.sectors_per_cluster,
            ops.reserved_sector_count,
            ops.fat_count,
            ops.media_type,
            ops.hidden_sector_count,
            ops.total_sectors_32,
            ops.sectors_per_fat_32,
            ops.root_cluster,
            ops.fs_info_sector,
            ops.boot_sector,
            ops.drive_number,
            ops.volume_id,
            ops.volume_label.as_ref().map(FixedByteStr::as_str),
        );
        let fat_sectors = ops.fat_count as u32 * ops.sectors_per_fat_32 as u32;
        let used_sectors = ops.reserved_sector_count as u32 + fat_sectors;
        let used_clusters = used_sectors / ops.sectors_per_cluster as u32;
        // Root cluster is used as well
        let fs_info = FsInfo::with_ops(&ops, used_clusters + 1);

        writer.write_bytes(0, &boot_sector.as_bytes())?;
        writer.write_bytes(
            ops.fs_info_sector as usize * ops.bytes_per_sector as usize,
            &fs_info.as_bytes(),
        )?;

        if ops.boot_sector != 0 {
            writer.write_bytes(
                ops.boot_sector as usize * ops.bytes_per_sector as usize,
                &boot_sector.as_bytes(),
            )?;
            writer.write_bytes(
                (ops.boot_sector as usize + 1) * ops.bytes_per_sector as usize,
                &fs_info.as_bytes(),
            )?;
        }

        let bs = match boot_sector.info() {
            BootSectorInfo::Fat32(info) => info,
        };
        let fs_info = fs_info.info();

        let fat = Fat32::new(
            ops.reserved_sector_count as usize * ops.bytes_per_sector as usize,
            fat_sectors as usize,
            ops.fat_count as usize,
            ops.bytes_per_sector as usize,
        );
        fat.init(writer);

        Ok(Self { bs, fs_info })
    }

    fn allocate_clusters(
        &mut self,
        writer: &mut dyn Writer,
        count: usize,
    ) -> Result<u32, FileSystemError> {
        let fat = Fat32::new(
            self.bs.reserved_sector_count as usize * self.bs.bytes_per_sector as usize,
            self.bs.sectors_per_fat as usize * self.bs.fat_count as usize,
            self.bs.fat_count as usize,
            self.bs.bytes_per_sector as usize,
        );
        Ok(fat.allocate_clusters(
            writer,
            count as u32,
            &mut self.fs_info.free_clusters,
            &mut self.fs_info.next_free_cluster,
        )?)
    }

    fn create_file_raw(
        &mut self,
        writer: &mut dyn Writer,
        path: &Path,
        attributes: FileAttributes,
        size: usize,
    ) -> Result<FileEntry, FileSystemError> {
        // Can't override root directory
        let parent = path.get_parent().ok_or(FileSystemError::NotFound)?;
        let stem = path.get_stem().ok_or(FileSystemError::NotFound)?;
        let directory_cluster = if parent.is_root() {
            self.bs.root_cluster
        } else {
            unimplemented!("Non root directory not supported")
        };

        let fat32 = Fat32::new(
            self.bs.reserved_sector_count as usize * self.bs.bytes_per_sector as usize,
            self.bs.sectors_per_fat as usize * self.bs.fat_count as usize,
            self.bs.fat_count as usize,
            self.bs.bytes_per_sector as usize,
        );
        let bytes_per_cluster =
            self.bs.bytes_per_sector as usize * self.bs.sectors_per_cluster as usize;
        let clusters = (size + bytes_per_cluster - 1) / bytes_per_cluster;
        let cluster = fat32.allocate_clusters(
            writer,
            clusters as u32,
            &mut self.fs_info.free_clusters,
            &mut self.fs_info.next_free_cluster,
        )?;

        let mut directory = Directory::new(
            (self.bs.reserved_sector_count as usize
                + self.bs.sectors_per_fat as usize * self.bs.fat_count as usize)
                * self.bs.bytes_per_sector as usize,
            self.bs.bytes_per_sector as usize * self.bs.sectors_per_cluster as usize,
        );
        let extension = stem.extension();
        let entry = FileEntry::new(
            stem.filename().as_str(),
            extension.as_ref().map(Path::as_str).unwrap_or(""),
            attributes,
            if attributes.contains(FileAttributes::DIRECTORY) {
                0
            } else {
                size as u32
            },
            cluster,
            FatTimeHighP::default(),
        );
        directory.write_entry(writer, directory_cluster, &entry)?;
        Ok(entry)
    }

    pub fn create_file(
        &mut self,
        writer: &mut dyn Writer,
        path: &Path,
        data: &[u8],
    ) -> Result<(), FileSystemError> {
        let entry = self.create_file_raw(writer, path, FileAttributes::ARCHIVE, data.len())?;
        let fat32 = Fat32::new(
            self.bs.reserved_sector_count as usize * self.bs.bytes_per_sector as usize,
            self.bs.sectors_per_fat as usize * self.bs.fat_count as usize * self.bs.bytes_per_sector as usize,
            self.bs.fat_count as usize,
            self.bs.bytes_per_sector as usize,
        );
        fat32.write_data(
            writer,
            self.bs.bytes_per_sector as usize,
            entry.cluster(),
            0,
            data,
        )?;
        Ok(())
    }

    pub fn flush(&mut self, writer: &mut dyn Writer) -> Result<(), FileSystemError> {
        // TODO: Flush the FS_INFO
        Ok(())
    }
}

pub struct DirectoryIter<'a> {
    reader: &'a mut dyn Reader,
    // The data buffer
    data: [u8; 512],
    fat_offset: usize,
    root_directory_offset: usize,
    /// Current cluster
    current_cluster: u32,
    /// Offset in the cluster, because we can only read 512 bytes at a time
    current_offset: usize,
    bytes_per_cluster: usize,
}

impl<'a> DirectoryIter<'a> {
    pub fn new(
        reader: &'a mut dyn Reader,
        fat_offset: usize,
        root_directory_offset: usize,
        cluster: u32,
        bytes_per_cluster: usize,
    ) -> Result<Self, FileSystemError> {
        let mut temp_buffer = [0u8; 512];
        let offset = bytes_per_cluster * (cluster - 2) as usize + root_directory_offset;
        reader.read_bytes(offset, &mut temp_buffer)?;

        Ok(Self {
            reader,
            data: temp_buffer,
            fat_offset,
            root_directory_offset,
            current_cluster: cluster,
            current_offset: 0,
            bytes_per_cluster,
        })
    }
}

impl Iterator for DirectoryIter<'_> {
    type Item = Result<FileEntryInfo, FileSystemError>;

    fn next(&mut self) -> Option<Result<FileEntryInfo, FileSystemError>> {
        // TODO: This doesn't read the next sector
        assert!(
            self.current_offset < 512,
            "Multi sector directory entries not supported"
        );
        let index = self.current_offset % 512;
        let file_entry =
            FileEntry::from_bytes(&self.data[index..index + size_of::<RawDirectoryEntry>()]);
        if file_entry.base_name().raw == [0; 8] {
            return None;
        }
        self.current_offset += size_of::<RawDirectoryEntry>();
        Some(Ok(file_entry.info()))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "std"))]
    compile_error!("feature \"std\" is required for tests");

    use super::*;

    #[test]
    fn test_create_fat32() {
        let mut data = Vec::with_capacity(65536 * 512);
        data.resize(65536 * 512, 0);
        let ops = Fat32Ops::recommended_config_for(65536);
        let mut fs = FatFs32::create_fat32(&mut data.as_mut_slice(), ops).unwrap();
        let file_data = "Hello World".as_bytes();
        fs.create_file(
            &mut data.as_mut_slice(),
            &Path::new("test.txt".into()),
            file_data,
        )
        .unwrap();

        for file in fs
            .list_dir(&mut data.as_mut_slice(), &Path::new("/".into()))
            .unwrap()
        {
            println!("{:?}", file);
        }

        dbg!(&fs);
        fs.flush(&mut data.as_mut_slice()).unwrap();
        drop(fs);
        std::fs::write("test.img", &data).unwrap();
    }
}

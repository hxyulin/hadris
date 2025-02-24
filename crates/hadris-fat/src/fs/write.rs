use crate::structures::{
    directory::{DirectoryReader, DirectoryWriter, FileAttributes, FileEntry},
    fat::{Fat32Reader, Fat32Writer},
    fs_info::FsInfo,
    time::FatTimeHighP,
};

pub use super::read::FileSystemError;
use super::read::FileSystemRead;
use hadris_core::{Reader, Writer};

pub struct FileSystemWrite {}

impl FileSystemWrite {
    fn get_parent<'a>(path: &'a str) -> Option<&'a str> {
        let mut index = path.len();
        if path.ends_with('/') {
            index -= 1;
        }
        while index > 0 {
            if path.as_bytes()[index - 1] == b'/' {
                if index == 1 {
                    return None;
                }
                return Some(&path[..index - 1]);
            }
            index -= 1;
        }
        None
    }

    fn is_root(&self, path: &str) -> bool {
        path.is_empty() || path == "/"
    }

    fn create_file_raw<T: Reader + Writer>(
        &self,
        fs_read: &FileSystemRead,
        writer: &mut T,
        path: &str,
        attributes: FileAttributes,
        size: u32,
    ) -> Result<(), FileSystemError> {
        // We find the parent directory
        let bs_info = fs_read.boot_sector_info(writer);
        let (parent_cluster, name) = if self.is_root(path) {
            (bs_info.root_cluster, path)
        } else {
            let parent = FileSystemWrite::get_parent(path);
            match parent {
                Some(parent) => {
                    let cluster = fs_read
                        .find_file_entry(writer, parent)?
                        .ok_or(FileSystemError::NotFound)?
                        .cluster();
                    let name = core::str::from_utf8(&parent.as_bytes()[parent.len()..]).unwrap();
                    (cluster, name)
                }
                None => (bs_info.root_cluster, path),
            }
        };

        let (name, extension) = if let Some(dot_index) = name.rfind('.') {
            (&name[..dot_index], &name[dot_index + 1..])
        } else {
            (name, "")
        };

        let fat_start = bs_info.reserved_sector_count as usize * bs_info.bytes_per_sector as usize;
        let fat_size = bs_info.sectors_per_fat as usize * bs_info.bytes_per_sector as usize;
        let fat_reader = Fat32Reader::new(fat_start, fat_size, bs_info.fat_count as usize);
        let fat_writer = Fat32Writer::new(bs_info.bytes_per_sector as usize);
        let sectors = (size as usize + bs_info.bytes_per_sector as usize - 1)
            / bs_info.bytes_per_sector as usize;
        let mut fs_info_buffer = [0u8; 512];
        writer.read_sector(bs_info.fs_info_sector as u32, &mut fs_info_buffer)?;
        let fs_info = FsInfo::from_bytes_mut(&mut fs_info_buffer);
        let mut free_count = fs_info.free_count;
        let mut next_free = fs_info.next_free;
        let cluster = fat_writer.allocate_clusters(
            &fat_reader,
            writer,
            sectors as u32,
            &mut free_count,
            &mut next_free,
        )?;
        fs_info.free_count = free_count;
        fs_info.next_free = next_free;
        writer.write_sector(bs_info.fs_info_sector as u32, &fs_info_buffer)?;

        // We create the directory
        let directory_reader = DirectoryReader::new(
            bs_info.root_cluster as usize,
            bs_info.bytes_per_sector as usize,
        );
        let mut directory = DirectoryWriter::new(directory_reader);
        let _ = directory.write_entry(
            writer,
            parent_cluster,
            FileEntry::new(
                name,
                extension,
                attributes,
                if attributes.contains(FileAttributes::DIRECTORY) {
                    0
                } else {
                    size
                },
                cluster,
                FatTimeHighP::default(),
            ),
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::read::tests::TestFs;

    #[test]
    fn test_get_parent() {
        let path = "/test/test/";
        let parent = FileSystemWrite::get_parent(path);
        assert_eq!(parent, Some("/test"));

        let path = "/test/test.txt";
        let parent = FileSystemWrite::get_parent(path);
        assert_eq!(parent, Some("/test"));

        let path = "/test/";
        let parent = FileSystemWrite::get_parent(path);
        assert_eq!(parent, None);

        let path = "/test";
        let parent = FileSystemWrite::get_parent(path);
        assert_eq!(parent, None);
    }

    #[test]
    fn test_create_file() {
        let path = "/test.txt";
        let attributes = FileAttributes::empty();
        let size = 0;

        let sectors = 1024;
        let mut data: Vec<u8> = Vec::with_capacity(1024 * 512);
        data.resize(sectors * 512, 0);
        let _ = TestFs::new(&mut data);
        let fs_reader = FileSystemRead {};
        let fs_writer = FileSystemWrite {};
        fs_writer.create_file_raw(&fs_reader, &mut data.as_mut_slice(), path, attributes, size).unwrap();
        drop(fs_writer);

        let info = fs_reader.get_file_info(&mut data.as_slice(), path).unwrap();
        assert_eq!(info.size(), size);
    }
}

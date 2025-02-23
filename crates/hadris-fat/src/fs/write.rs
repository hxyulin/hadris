pub use super::read::FileSystemError;
use super::read::FileSystemRead;
use hadris_core::{Reader, Writer};

pub struct FileSystemWrite {}

impl FileSystemWrite {
    fn get_parent<'a>(&self, path: &'a str) -> Option<&'a str> {
        // We find the last slash, and we return everything before it
        if let Some(idx) = path.rfind('/') {
            Some(&path[..idx])
        } else {
            None
        }
    }

    fn is_root(&self, path: &str) -> bool {
        path.is_empty() || path == "/"
    }

    pub fn create_directory<T: Reader + Writer>(
        &self,
        fs_read: &FileSystemRead,
        reader: &mut T,
        path: &str,
    ) -> Result<(), FileSystemError> {
        //use crate::structures::directory::{DirectoryReader, DirectoryWriter};
        // We find the parent directory
        let parent = self.get_parent(path);
        let parent_cluster = if self.is_root(path) {
            fs_read.root_directory_cluster(reader)
        } else {
            fs_read
                .find_file_entry(reader, path)?
                .ok_or(FileSystemError::NotFound)?
                .cluster()
        };

        // We create the directory
        //let directory = DirectoryWriter::new(DirectoryReader::new(
        //));

        todo!()
    }
}

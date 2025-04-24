use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum FileData {
    Data(Vec<u8>),
    File(PathBuf),
    /// A list of files in the directory, relative to the directory
    Directory(Vec<String>),
}

impl FileData {
    pub fn get_data(&self) -> Vec<u8> {
        match self {
            Self::Data(data) => data.clone(),
            Self::File(path) => std::fs::read(path).unwrap(),
            Self::Directory(_) => panic!("Cannot get data from a directory"),
        }
    }

    pub fn get_children(&self) -> Vec<String> {
        match self {
            Self::Directory(children) => children.clone(),
            _ => panic!("Cannot get children of a file"),
        }
    }

    pub fn add_child(&mut self, child: String) {
        match self {
            Self::Directory(children) => children.push(child),
            _ => panic!("Cannot add child to a file"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct File {
    pub path: String,
    pub data: FileData,
}

impl File {
    pub fn is_directory(&self) -> bool {
        matches!(self.data, FileData::Directory(_))
    }

    pub fn get_data(&self) -> Vec<u8> {
        self.data.get_data()
    }

    pub fn get_children(&self) -> Vec<String> {
        self.data.get_children()
    }

    pub fn add_child(&mut self, child: String) {
        self.data.add_child(child);
    }
}

#[derive(Debug, Clone)]
pub struct FileInput {
    /// A flat list of files
    /// nested files should be written with the parent directory path:
    /// ```text
    /// /path/to/file
    /// ```
    files: Vec<File>,
}

impl FileInput {
    pub const fn empty() -> Self {
        Self { files: Vec::new() }
    }

    #[cfg(feature = "std")]
    pub fn from_fs<P: AsRef<std::path::Path>>(root: P) -> Result<FileInput, std::io::Error> {
        let root = root.as_ref();
        assert!(root.is_dir(), "File {} is not a directory", root.display());
        let mut files = vec![File {
            path: "".to_string(),
            data: FileData::Directory(Vec::new()),
        }];
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut childrens = Vec::new();
            let children = std::fs::read_dir(&dir)?;
            for child in children {
                let child = child?;
                childrens.push(child.file_name().to_str().unwrap().to_string());

                let name = child
                    .path()
                    .strip_prefix(&root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                if child.file_type()?.is_dir() {
                    files.push(File {
                        path: name,
                        data: FileData::Directory(Vec::new()),
                    });
                    stack.push(child.path());
                    continue;
                }

                let path = child.path();
                files.push(File {
                    path: name,
                    data: FileData::File(path),
                });
            }
            let dir = dir.strip_prefix(&root).unwrap();
            let dir_name = dir.to_str().unwrap();
            let dir = files.iter_mut().find(|f| f.path == dir_name).unwrap();
            dir.data = FileData::Directory(childrens);
        }

        Ok(Self { files })
    }

    /// Splits the files into two lists,
    /// one containing files,
    /// and one containing directories
    pub fn split(mut self) -> (Vec<File>, Vec<File>) {
        let mut dirs: Vec<File> = Vec::new();
        self.files.retain(|f| {
            if let FileData::Directory(_) = f.data {
                dirs.push(f.clone());
                false
            } else {
                true
            }
        });
        (dirs, self.files)
    }

    pub fn append(&mut self, file: File) {
        // TODO: Support adding nested files
        let parent = self.get_mut("").unwrap();
        parent.add_child(file.path.clone());
        self.files.push(file);
    }

    pub fn get(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.path == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut File> {
        self.files.iter_mut().find(|f| f.path == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn iter(&self) -> FileIter {
        FileIter {
            files: &self.files,
            index: 0,
        }
    }
}

impl<'a> IntoIterator for &'a FileInput {
    type Item = &'a File;
    type IntoIter = FileIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct FileIter<'a> {
    files: &'a [File],
    index: usize,
}

impl<'a> Iterator for FileIter<'a> {
    type Item = &'a File;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.files.len() {
            None
        } else {
            let file = &self.files[self.index];
            self.index += 1;
            Some(file)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_file_input() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path();
        let boot_dir = root_path.join("BOOT/GRUB");
        let efi_dir = root_path.join("EFI");
        fs::create_dir_all(&boot_dir).unwrap();
        fs::create_dir_all(&efi_dir).unwrap();
        let grub_cfg = boot_dir.join("GRUB.CFG");
        fs::write(&grub_cfg, "test").unwrap();
        let efi_cfg = efi_dir.join("BOOTX64.EFI");
        fs::write(&efi_cfg, "test2").unwrap();

        let fs = FileInput::from_fs(root.into_path()).unwrap();
        let grub_cfg = fs
            .files
            .iter()
            .find(|f| f.path == "BOOT/GRUB/GRUB.CFG")
            .unwrap();
        let efi_cfg = fs
            .files
            .iter()
            .find(|f| f.path == "EFI/BOOTX64.EFI")
            .unwrap();
    }
}

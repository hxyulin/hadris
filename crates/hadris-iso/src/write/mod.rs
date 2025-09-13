use core::fmt;
use std::collections::HashMap;

mod writer;

use crate::{
    LogicalSector,
    directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef, FileFlags},
    read::PathSeparator,
    volume::{PrimaryVolumeDescriptor, VolumeDescriptor, VolumeDescriptorList},
    write::writer::{EntryType, WrittenDirectory, WrittenFile, WrittenFiles},
};
use bytemuck::Zeroable;
use hadris_io::{self as io, Read, Seek, SeekFrom, Write};

use alloc::{collections::VecDeque, string::String, vec, vec::Vec};

pub mod options;
use options::FormatOptions;

struct Cursor<DATA: Seek> {
    data: DATA,
    sector_size: usize,
}

impl<DATA: Read + Seek> Read for Cursor<DATA> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.read(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.data.read_exact(buf)
    }
}

impl<DATA: Seek> Seek for Cursor<DATA> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.data.seek(pos)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        self.data.stream_position()
    }

    fn seek_relative(&mut self, offset: i64) -> io::Result<()> {
        self.data.seek_relative(offset)
    }
}

impl<DATA: Seek> Cursor<DATA> {
    pub fn new(data: DATA, sector_size: usize) -> Self {
        Self { data, sector_size }
    }

    pub fn pad_align_sector(&mut self) -> io::Result<LogicalSector> {
        let stream_pos = self.stream_position()?;
        let sector_size_minus_one = self.sector_size as u64 - 1;
        let aligned_pos = (stream_pos + sector_size_minus_one) & !sector_size_minus_one;
        if aligned_pos != stream_pos {
            self.seek(SeekFrom::Start(aligned_pos))?;
        }
        Ok(LogicalSector(
            (aligned_pos / sector_size_minus_one) as usize,
        ))
    }

    pub fn seek_sector(&mut self, sector: LogicalSector) -> io::Result<u64> {
        self.seek(SeekFrom::Start(sector.0 as u64 * self.sector_size as u64))
    }
}

impl<DATA: Write + Seek> Write for Cursor<DATA> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.data.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.data.write_all(buf)
    }
}

impl<DATA: Seek> fmt::Debug for Cursor<DATA> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cursor").finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FileConversionError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Path {0:?} is not a valid UTF-8 string")]
    InvalidUtf8Path(std::path::PathBuf),
}

impl InputFiles {
    pub fn from_fs(
        root_path: &std::path::Path,
        path_separator: PathSeparator,
    ) -> Result<Self, FileConversionError> {
        if !root_path.is_dir() {
            return Err(FileConversionError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                alloc::format!("Root path '{:?}' is not a directory", root_path),
            )));
        }

        let children = read_directory_recursively(root_path)?;

        Ok(Self {
            path_separator,
            files: children,
        })
    }
}

/// Recursively reads a directory and converts its contents into a vector of `File` enums.
fn read_directory_recursively(
    current_path: &std::path::Path,
) -> Result<Vec<File>, FileConversionError> {
    use alloc::string::ToString;
    let mut children_files: Vec<File> = Vec::new();

    for entry_result in std::fs::read_dir(current_path)? {
        let entry = entry_result?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .ok_or_else(|| FileConversionError::InvalidUtf8Path(path.clone()))?
            .to_string();

        if path.is_file() {
            let contents = std::fs::read(&path)?;
            children_files.push(File::File { name, contents });
        } else if path.is_dir() {
            let grand_children = read_directory_recursively(&path)?;
            children_files.push(File::Directory {
                name,
                children: grand_children,
            });
        }
        // Else: ignore other file types (e.g., symlinks, pipes) for now
    }

    // Sort files and directories for consistent ISO ordering (optional, but good practice)
    children_files.sort_by_key(|f| f.name().to_ascii_lowercase());

    Ok(children_files)
}

pub struct InputFiles {
    pub path_separator: PathSeparator,
    pub files: Vec<File>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum File {
    File { name: String, contents: Vec<u8> },
    Directory { name: String, children: Vec<File> },
}

impl core::fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("File");
        match self {
            Self::Directory { name, children } => {
                dbg.field("name", name);
                dbg.field("children", children);
            }
            Self::File { name, contents } => {
                dbg.field("name", name);
                dbg.field("data_len", &contents.len());
            }
        }
        dbg.finish()
    }
}

impl File {
    pub fn name(&self) -> &str {
        match self {
            File::File { name, .. } => &name,
            File::Directory { name, .. } => &name,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IsoCreationError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type IsoCreationResult<T> = Result<T, IsoCreationError>;

pub struct IsoImageWriter<DATA: Read + Write + Seek> {
    data: Cursor<DATA>,
    files: InputFiles,
    entry_types: Vec<EntryType>,
    ops: FormatOptions,
}

impl<DATA: Read + Write + Seek> IsoImageWriter<DATA> {
    pub fn format_new(data: DATA, files: InputFiles, ops: FormatOptions) -> IsoCreationResult<()> {
        let mut writer = Self::new(data, files, ops);
        writer.write_volume_descriptors()?;
        let root_dir = writer.write_files()?;
        writer.finalize_volume_descriptors(root_dir)?;
        Ok(())
    }

    fn new(data: DATA, files: InputFiles, ops: FormatOptions) -> Self {
        let mut entry_types = Vec::new();
        entry_types.push(ops.features.filenames.into());
        if ops.features.long_filenames {
            entry_types.push(EntryType::Level3);
        }
        if let Some(joliet) = ops.features.joliet {
            entry_types.push(joliet.into());
        }

        Self {
            data: Cursor::new(data, ops.sector_size),
            files,
            ops,
            entry_types,
        }
    }

    const VOLUME_DESCRIPTOR_SET_START: LogicalSector = LogicalSector(16);

    fn write_volume_descriptors(&mut self) -> io::Result<()> {
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START)?;
        let mut volume_descriptors = VolumeDescriptorList::empty();
        let pvd = PrimaryVolumeDescriptor::new(&self.ops.volume_name, 0);
        volume_descriptors.push(VolumeDescriptor::Primary(pvd));
        volume_descriptors.write(&mut self.data)?;
        Ok(())
    }

    fn finalize_volume_descriptors(
        &mut self,
        root_dirs: HashMap<EntryType, DirectoryRef>,
    ) -> io::Result<()> {
        let base_type = self
            .entry_types
            .iter()
            .find(|ty| **ty == EntryType::Level1 || **ty == EntryType::Level2)
            .expect("failed to find base Level");
        let root_dir = root_dirs.get(base_type).unwrap();
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START)?;
        let mut pvd = PrimaryVolumeDescriptor::zeroed();
        self.data.read_exact(bytemuck::bytes_of_mut(&mut pvd))?;
        pvd.dir_record.header.extent.write(root_dir.extent.0 as u32);
        pvd.dir_record.header.data_len.write(root_dir.size as u32);
        let sector = self.data.pad_align_sector()?;
        pvd.volume_space_size.write(sector.0 as u32);
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START)?;
        self.data.write_all(bytemuck::bytes_of(&pvd))?;
        Ok(())
    }

    fn write_files(&mut self) -> io::Result<HashMap<EntryType, DirectoryRef>> {
        let roots = {
            let mut written_files = WrittenFiles::new();
            let level = self.ops.features.filenames;
            let mut files = FileTreeWalker::new(&self.files);
            let mut current_dir = written_files.root_dir();
            while let Some(file) = files.next() {
                match file {
                    TreeWalkerItem::EnterDirectory(dir) => {
                        let name = dir.name();
                        let dir = written_files.get_mut(&current_dir);
                        current_dir.push(dir.push_dir(name));
                    }
                    TreeWalkerItem::ExitDirectory(_dir) => {
                        let dir = written_files.get_mut(&current_dir);
                        Self::write_directory(&mut self.data, level.into(), dir)?;
                        current_dir.pop();
                    }
                    TreeWalkerItem::File(file) => {
                        if let File::File { name, contents } = file {
                            let start = self.data.pad_align_sector()?;
                            self.data.write_all(&contents)?;
                            let dir = written_files.get_mut(&current_dir);
                            dir.files.push(WrittenFile {
                                name,
                                entry: DirectoryRef {
                                    extent: start,
                                    size: contents.len(),
                                },
                            });
                        }
                    }
                };
            }

            // Write root directory
            let dir = written_files.get_mut(&current_dir);
            for ty in &self.entry_types {
                Self::write_directory(&mut self.data, *ty, dir)?;
            }

            written_files.root_refs().clone()
        };
        for (_, root) in &roots {
            self.update_directory(*root, *root)?;
        }

        Ok(roots)
    }

    fn update_directory(
        &mut self,
        parent: DirectoryRef,
        directory: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent)?;

        DirectoryRecord::new(b"\x00", directory, FileFlags::DIRECTORY).write(&mut self.data)?;
        DirectoryRecord::new(b"\x01", parent, FileFlags::DIRECTORY).write(&mut self.data)?;

        let end = self.data.stream_position()?;
        let mut offset = (end - start) as usize;

        loop {
            if offset >= directory.size {
                break;
            }
            self.data.seek(SeekFrom::Start(start + offset as u64))?;
            let mut header = DirectoryRecordHeader::zeroed();
            self.data.read_exact(bytemuck::bytes_of_mut(&mut header))?;
            if header.len == 0 {
                break;
            }

            let mut bytes = vec![0; header.len as usize - size_of::<DirectoryRecordHeader>()];
            self.data.read_exact(&mut bytes)?;
            offset += header.len as usize;

            if FileFlags::from_bits_truncate(header.flags).contains(FileFlags::DIRECTORY) {
                let record = DirectoryRef {
                    extent: LogicalSector(header.extent.read() as usize),
                    size: header.data_len.read() as usize,
                };
                self.update_directory(directory, record)?;
            }
        }

        Ok(())
    }

    fn write_directory(
        data: &mut Cursor<DATA>,
        ty: EntryType,
        dir: &mut WrittenDirectory,
    ) -> io::Result<()> {
        let start = data.pad_align_sector()?;
        // Current Directory Entry (unfilled)
        DirectoryRecord::with_len(1).write(&mut *data)?;
        // Parent Directory Entry (unfilled)
        DirectoryRecord::with_len(1).write(&mut *data)?;

        for directory in &dir.dirs {
            let WrittenDirectory { name, entries, .. } = directory;
            let flags = FileFlags::DIRECTORY;
            let converted_name = ty.convert_name(name);
            let record =
                DirectoryRecord::new(converted_name.as_bytes(), *entries.get(&ty).unwrap(), flags);
            record.write(&mut *data)?;
        }

        for file in &dir.files {
            let WrittenFile { name, entry } = file;
            let flags = FileFlags::empty();
            let converted_name = ty.convert_name(name);
            let record = DirectoryRecord::new(converted_name.as_bytes(), *entry, flags);
            record.write(&mut *data)?;
        }

        let end = data.pad_align_sector()?;
        let size = (end.0 - start.0) * data.sector_size;
        dir.entries.insert(
            ty,
            DirectoryRef {
                extent: start,
                size,
            },
        );
        Ok(())
    }
}

#[allow(dead_code)]
struct FileTreeWalker<'a> {
    input_files: &'a InputFiles,
    stack: VecDeque<StackFrame<'a>>,
}

enum StackFrame<'a> {
    Node(&'a File),
    DirExit(&'a File),
}

#[derive(Debug, PartialEq, Eq)]
enum TreeWalkerItem<'a> {
    EnterDirectory(&'a File),
    File(&'a File),
    ExitDirectory(&'a File),
}

impl<'a> FileTreeWalker<'a> {
    pub fn new(input: &'a InputFiles) -> Self {
        let mut stack = VecDeque::new();
        for file in input.files.iter().rev() {
            stack.push_back(StackFrame::Node(file));
        }
        FileTreeWalker {
            input_files: input,
            stack,
        }
    }
}

impl<'a> Iterator for FileTreeWalker<'a> {
    type Item = TreeWalkerItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(frame) = self.stack.pop_back() {
            match frame {
                StackFrame::Node(file) => {
                    match file {
                        File::File { .. } => {
                            return Some(TreeWalkerItem::File(file));
                        }
                        File::Directory { children, .. } => {
                            // Yield that we are entering this directory (pre-order event)
                            let current_dir = file;

                            // Push an Exit frame to signal leaving this directory later
                            self.stack.push_back(StackFrame::DirExit(current_dir));

                            // Push children in reverse order for DFS
                            for child in children.iter().rev() {
                                self.stack.push_back(StackFrame::Node(child));
                            }

                            return Some(TreeWalkerItem::EnterDirectory(current_dir));
                        }
                    }
                }
                StackFrame::DirExit(dir) => {
                    return Some(TreeWalkerItem::ExitDirectory(dir));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Import items from the outer module
    use alloc::vec;

    #[test]
    fn test_depth_first_tree_walk_iterator() {
        // Define a test file hierarchy
        let file_a = File::File {
            name: String::from("root/dir1/fileA.txt"),
            contents: Vec::new(),
        };
        let file_b = File::File {
            name: String::from("root/dir1/fileB.txt"),
            contents: Vec::new(),
        };
        let file_c = File::File {
            name: String::from("root/fileC.txt"),
            contents: Vec::new(),
        };
        let file_d = File::File {
            name: String::from("root/dir2/fileD.txt"),
            contents: Vec::new(),
        };
        let file_e = File::File {
            name: String::from("root/dir2/subdir/fileE.txt"),
            contents: Vec::new(),
        };

        let subdir_node = File::Directory {
            name: String::from("root/dir2/subdir"),
            children: vec![file_e.clone()],
        };

        let dir1_node = File::Directory {
            name: String::from("root/dir1"),
            children: vec![file_a.clone(), file_b.clone()],
        };

        let dir2_node = File::Directory {
            name: String::from("root/dir2"),
            children: vec![
                file_d.clone(),
                subdir_node.clone(), // Subdirectory
            ],
        };

        let root_level_files = vec![dir1_node.clone(), file_c.clone(), dir2_node.clone()];

        let input_tree = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: root_level_files,
        };

        // Create the iterator
        let walker = FileTreeWalker::new(&input_tree);

        // Define the expected sequence of events (depth-first, pre-order for Enter, post-order for Exit)
        let expected_sequence = vec![
            TreeWalkerItem::EnterDirectory(&dir1_node),   // Enter dir1
            TreeWalkerItem::File(&file_a),                // Process fileA
            TreeWalkerItem::File(&file_b),                // Process fileB
            TreeWalkerItem::ExitDirectory(&dir1_node),    // Exit dir1
            TreeWalkerItem::File(&file_c),                // Process fileC
            TreeWalkerItem::EnterDirectory(&dir2_node),   // Enter dir2
            TreeWalkerItem::File(&file_d),                // Process fileD
            TreeWalkerItem::EnterDirectory(&subdir_node), // Enter subdir
            TreeWalkerItem::File(&file_e),                // Process fileE
            TreeWalkerItem::ExitDirectory(&subdir_node),  // Exit subdir
            TreeWalkerItem::ExitDirectory(&dir2_node),    // Exit dir2
        ];

        // Collect all items from the iterator
        let actual_sequence: Vec<TreeWalkerItem> = walker.collect();

        // Assert that the actual sequence matches the expected sequence
        assert_eq!(actual_sequence, expected_sequence);
    }
}

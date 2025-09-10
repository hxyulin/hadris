use core::fmt;

use crate::{
    LogicalSector,
    directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef, FileFlags},
    options::FormatOptions,
    read::PathSeparator,
    types::IsoStringD,
    volume::{PrimaryVolumeDescriptor, VolumeDescriptor, VolumeDescriptorList},
};
use bytemuck::Zeroable;
use hadris_io::{self as io, Read, Seek, SeekFrom, Write};

use alloc::{collections::VecDeque, format, string::String, vec, vec::Vec};

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

pub struct InputFiles {
    pub path_separator: PathSeparator,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum File {
    File { name: String, contents: Vec<u8> },
    Directory { name: String, children: Vec<File> },
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

#[derive(Debug)]
pub struct WrittenFile {
    original_name: String,
    parent: String,
    is_directory: bool,
    start: LogicalSector,
    size: usize,
}

pub struct IsoImageWriter<DATA: Read + Write + Seek> {
    data: Cursor<DATA>,
    files: InputFiles,
    written_files: Vec<WrittenFile>,
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
        Self {
            data: Cursor::new(data, ops.sector_size),
            files,
            written_files: Vec::new(),
            ops,
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

    fn finalize_volume_descriptors(&mut self, root_dir: DirectoryRef) -> io::Result<()> {
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

    fn write_files(&mut self) -> io::Result<DirectoryRef> {
        let level = self.ops.features.filenames;
        let mut files = FileTreeWalker::new(&self.files);
        let mut prefix = String::new();
        while let Some(file) = files.next() {
            match file {
                TreeWalkerItem::EnterDirectory(dir) => {
                    if !prefix.is_empty() {
                        prefix.push(self.files.path_separator.as_char());
                    }
                    prefix.push_str(dir.name());
                }
                TreeWalkerItem::ExitDirectory(dir) => {
                    let dirname = prefix.clone();
                    prefix.truncate(prefix.len() - dir.name().len());
                    // We either pop the leading slash, or it is empty already, and we ignore the
                    // None variant
                    _ = prefix.pop();
                    let parent = prefix.clone();
                    Self::write_directory(
                        &mut self.data,
                        &mut self.written_files,
                        dirname,
                        parent,
                    )?;
                }
                TreeWalkerItem::File(file) => {
                    if let File::File { name, contents } = file {
                        let parent = prefix.clone();

                        let fullname = format!("{}/{}", prefix, name);
                        let start = self.data.pad_align_sector()?;
                        self.data.write_all(&contents)?;
                        self.written_files.push(WrittenFile {
                            original_name: fullname,
                            parent,
                            is_directory: false,
                            start,
                            size: contents.len(),
                        });
                    }
                }
            };
        }

        let dir_ref = Self::write_directory(
            &mut self.data,
            &mut self.written_files,
            String::new(),
            String::new(),
        )?;

        self.update_directory(dir_ref, dir_ref)?;

        Ok(dir_ref)
    }

    fn update_directory(
        &mut self,
        parent: DirectoryRef,
        directory: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent)?;

        DirectoryRecord::new(
            IsoStringD::from_utf8("\x00"),
            directory,
            FileFlags::DIRECTORY,
        )
        .write(&mut self.data)?;

        DirectoryRecord::new(IsoStringD::from_utf8("\x01"), parent, FileFlags::DIRECTORY)
            .write(&mut self.data)?;
        let end = self.data.stream_position()?;
        let mut offset = (end - start) as usize;

        loop {
            if offset >= directory.size {
                break;
            }
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
        written_files: &mut Vec<WrittenFile>,
        dirname: String,
        parent: String,
    ) -> io::Result<DirectoryRef> {
        let start = data.pad_align_sector()?;
        // Current Directory Entry (unfilled)
        DirectoryRecord::with_len(1).write(&mut *data)?;
        // Parent Directory Entry (unfilled)
        DirectoryRecord::with_len(1).write(&mut *data)?;
        for entry in written_files.iter().filter(|f| f.parent == dirname) {
            let basename = entry.original_name.strip_prefix(&dirname).unwrap();
            let mut flags = FileFlags::empty();
            if entry.is_directory {
                flags |= FileFlags::DIRECTORY;
            }
            let record = DirectoryRecord::new(
                IsoStringD::from_utf8(basename),
                DirectoryRef {
                    extent: entry.start,
                    size: entry.size,
                },
                flags,
            );
            record.write(&mut *data)?;
        }

        let end = data.pad_align_sector()?;
        let size = (end.0 - start.0) * data.sector_size;
        written_files.push(WrittenFile {
            original_name: dirname,
            parent,
            is_directory: true,
            start: start,
            size,
        });
        Ok(DirectoryRef {
            extent: start,
            size,
        })
    }
}

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

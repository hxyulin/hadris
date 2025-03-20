//! The iso-rs crate provides a library for creating and reading ISO images
//! The library is designed to be flexible and easy to use, ideally supporting
//! all the futures that xorriso will support.
//! The library currently requires `std`, but it is planned to support no_std and even non-allocator
//! environments in the future.
//!
//! To create a basic ISO image, you can use the `FormatOptions` struct:
//! ```no_run
//! use hadris_iso::{PartitionOptions, IsoImage, FileInput, FormatOptions};
//! use std::fs::File;
//! use std::io::Write;
//!
//! let options = FormatOptions::new()
//!     .with_files(FileInput::from_fs(std::path::PathBuf::from("path/to/files")).unwrap())
//!     .with_format_options(PartitionOptions::PROTECTIVE_MBR);
//! let mut file = File::create("path/to/image.iso").unwrap();
//! let mut iso = IsoImage::format_new(&mut file, options).unwrap();
//! ```

// We keep boot separate since it is a seperate specification
#[cfg(feature = "el-torito")]
pub mod boot;
#[cfg(feature = "el-torito")]
pub use boot::*;

mod directory;
mod file;
mod options;
mod path;
mod types;
mod volume;

pub use directory::*;
pub use file::*;
pub use options::*;
pub use path::*;

use std::{
    collections::BTreeMap,
    fmt::Debug,
    io::{Read, Seek, SeekFrom, Write},
};
pub use types::*;
pub use volume::*;

pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T: Read + Write + Seek> ReadWriteSeek for T {}

#[derive(Debug, thiserror::Error)]
pub enum IsoImageError {
    /// The image is too small, check [`FormatOptions::image_len()`] for the minimum size
    #[error("The image is too small, expected at least {0}b, got {1}b")]
    ImageTooSmall(u64, u64),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct IsoImage<'a, T: ReadWriteSeek> {
    data: &'a mut T,

    volume_descriptors: VolumeDescriptorList,
    root_directory: DirectoryRef,
    path_table: PathTableRef,
}

impl<'a, T: ReadWriteSeek> IsoImage<'a, T> {
    pub fn format_file<P>(path: P, options: FormatOptions) -> Result<(), IsoImageError>
    where
        P: AsRef<std::path::Path>,
    {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .unwrap();
        let (min, max) = options.image_len();
        log::trace!("Calculate minimum and maximum size of image: {min}b to {max}b");
        file.set_len(max).unwrap();
        IsoImage::format_new(&mut file, options)
    }
    /// Formats a new ISO image,
    /// for a more convenient API, see [`Self::format_file`] for [`std::fs::File`]
    pub fn format_new(data: &'a mut T, mut ops: FormatOptions) -> Result<(), IsoImageError> {
        let size_bytes = data.seek(SeekFrom::End(0))?;
        let min_size = ops.image_len().0;
        if size_bytes < min_size {
            return Err(IsoImageError::ImageTooSmall(min_size, size_bytes));
        }

        let size_sectors = size_bytes / 2048;
        log::trace!(
            "Started formatting ISO image with {} sectors ({}) bytes)",
            size_sectors,
            size_bytes
        );

        if ops.format.contains(PartitionOptions::PROTECTIVE_MBR) {
            data.seek(SeekFrom::Start(0))?;
            data.write_all(bytemuck::bytes_of(&ProtectiveMBR::new(size_sectors as u32)))?;
        }

        let mut volume_descriptors = VolumeDescriptorList::empty();

        volume_descriptors.push(VolumeDescriptor::Primary(PrimaryVolumeDescriptor::new(
            size_sectors as u32,
        )));

        #[cfg(feature = "el-torito")]
        if let Some(boot_ops) = &ops.boot {
            log::trace!("Adding boot record to volume descriptors");
            volume_descriptors.push(VolumeDescriptor::BootRecord(
                BootRecordVolumeDescriptor::new(0),
            ));
            assert!(
                ops.files.contains(&boot_ops.default.boot_image_path),
                "Boot image path not found in files"
            );

            if boot_ops.write_boot_catalogue {
                log::trace!("Appending boot catalogue to file list");
                ops.files.append(file::File {
                    path: "boot.catalog".to_string(),
                    // TODO: We need to make this dynamic
                    data: file::FileData::Data(vec![0; 32 * 4]),
                });
            }
        }

        let mut current_index: u64 = 16 * 2048;
        current_index += volume_descriptors.size_required() as u64;
        data.seek(SeekFrom::Start(current_index as u64))?;

        let mut file_writer = FileWriter::new(data, ops.files);
        let (root_dir, path_table) = file_writer.write()?;

        {
            log::trace!("Updating primary volume descriptor");
            let pvd = volume_descriptors.primary_mut();
            pvd.dir_record.header.extent.write(root_dir.offset as u32);
            pvd.dir_record.header.data_len.write(root_dir.size as u32);
            pvd.path_table_size.write(path_table.size as u32);
            pvd.type_l_path_table.set(path_table.offset as u32);
            pvd.type_m_path_table
                .set(path_table.offset as u32 + (path_table.size / 2048) as u32);
        }

        #[cfg(feature = "el-torito")]
        if let Some(boot_ops) = ops.boot {
            // TODO: If we support nested files, we need to find them from the Path table, and not
            // the root directory
            let mut root_dir = IsoDirectory {
                reader: data,
                directory: root_dir.clone(),
            };

            // TODO: Support more than just the default entry
            let mut catalog = BootCatalogue::default();

            let (_, file) = root_dir
                .entries()?
                .iter()
                .find(|(_idx, e)| e.name.to_str() == boot_ops.default.boot_image_path.as_str())
                .expect("Could not find the boot image path in ISO filesystem")
                .clone();
            let (_, catalog_file) = root_dir
                .entries()?
                .iter()
                .find(|(_idx, e)| e.name.to_str() == "boot.catalog")
                .expect("Could not find the boot catalogue in ISO filesystem")
                .clone();

            let current_index = Self::align(data)?;

            let boot_image_lba = file.header.extent.read();
            catalog.set_default_entry(BootSectionEntry::new(
                boot_ops.default.emulation,
                0,
                boot_ops.default.load_size,
                boot_image_lba,
            ));

            if boot_ops.default.boot_info_table {
                let mut checksum = 0u32;
                let mut buffer = [0u8; 4];
                data.seek(SeekFrom::Start(
                    file.header.extent.read() as u64 * 2048 + 64,
                ))?;
                for _ in (64..file.header.data_len.read()).step_by(4) {
                    data.read_exact(&mut buffer)?;
                    checksum = checksum.wrapping_add(u32::from_le_bytes(buffer));
                }
                let byte_offset = boot_image_lba * 2048;
                let table = BootInfoTable {
                    iso_start: U32::new(16),
                    file_lba: U32::new(file.header.extent.read()),
                    file_len: U32::new(file.header.data_len.read()),
                    checksum: U32::new(checksum),
                };

                const TABLE_OFFSET: u64 = 8;
                data.seek(SeekFrom::Start(byte_offset as u64 + TABLE_OFFSET))?;
                data.write_all(bytemuck::bytes_of(&table))?;
            }

            // UNTESTED
            if boot_ops.default.grub2_boot_info {
                // The GRUB2 boot info wants the start of the image file in 512 blocks + 5
                let value = file.header.extent.read() * 4 + 5;
                // It is from byte 2548 to 2555
                data.seek(SeekFrom::Start(
                    file.header.extent.read() as u64 * 2048 + 2548,
                ))?;
                data.write_all(&value.to_le_bytes())?;
            }

            data.seek(SeekFrom::Start(current_index))?;

            let catalogue_start = Self::align(data)? / 2048;
            volume_descriptors
                .boot_record_mut()
                .unwrap()
                .catalog_ptr
                .set(catalogue_start as u32);
            catalog.write(data)?;
            let end = Self::align(data)?;

            data.seek(SeekFrom::Start(
                catalog_file.header.extent.read() as u64 * 2048,
            ))?;
            assert!(catalog_file.header.data_len.read() as usize >= catalog.size());
            catalog.write(data)?;
            data.seek(SeekFrom::Start(end))?;
        }
        let end = Self::align(data)?;

        data.seek(SeekFrom::Start(16 * 2048))?;
        volume_descriptors.write(data)?;

        // We need to be at the end of the image
        data.seek(SeekFrom::Start(end))?;
        Ok(())
    }

    pub fn new(data: &'a mut T) -> Result<Self, std::io::Error> {
        data.seek(SeekFrom::Start(16 * 2048))?;
        let volume_descriptors = VolumeDescriptorList::parse(data)?;

        let pvd = volume_descriptors.primary();
        #[cfg(feature = "el-torito")]
        if let Some(boot) = volume_descriptors.boot_record() {
            data.seek(SeekFrom::Start(boot.catalog_ptr.get() as u64 * 2048))?;
            let _catalogue = BootCatalogue::parse(data)?;
            // At the moment we dont support anything with a boot catalogue
        }

        let root_entry = pvd.dir_record;
        let root_directory = DirectoryRef {
            offset: root_entry.header.extent.read() as u64,
            size: root_entry.header.data_len.read() as u64,
        };

        let path_table = PathTableRef {
            lpath_table_offset: pvd.type_l_path_table.get() as u64,
            mpath_table_offset: pvd.type_m_path_table.get() as u64,
            size: pvd.path_table_size.read() as u64,
        };

        Ok(Self {
            data,

            volume_descriptors,
            root_directory,
            path_table,
        })
    }

    pub fn root_directory(&mut self) -> IsoDirectory<T> {
        IsoDirectory {
            reader: &mut self.data,
            directory: self.root_directory,
        }
    }

    pub fn path_table(&mut self) -> IsoPathTable<T> {
        IsoPathTable {
            reader: &mut self.data,
            path_table: self.path_table,
        }
    }

    fn current_sector(data: &mut T) -> usize {
        let seek = data.seek(std::io::SeekFrom::Current(0)).unwrap();
        assert!(seek % 2048 == 0, "Seek must be a multiple of 2048");
        (seek / 2048) as usize
    }

    fn align(data: &mut T) -> Result<u64, std::io::Error> {
        let current_seek = data.seek(std::io::SeekFrom::Current(0))?;
        let padded_end = (current_seek + 2047) & !2047;
        data.seek(std::io::SeekFrom::Start(padded_end))?;
        Ok(padded_end)
    }
}

#[derive(Debug)]
struct FileWriter<'a, W: ReadWriteSeek> {
    writer: &'a mut W,

    dirs: Vec<file::File>,
    files: Vec<file::File>,

    /// The first element is whether the file is a directory
    written_files: BTreeMap<String, (bool, DirectoryRef)>,
}

impl<'a, W: ReadWriteSeek> FileWriter<'a, W> {
    pub fn new(writer: &'a mut W, files: FileInput) -> Self {
        log::trace!("Started writing files");
        let (mut dirs, mut files) = files.split();

        log::trace!("Sorting directories by depth");
        Self::sort_by_depth(&mut dirs);
        Self::sort_by_depth(&mut files);

        Self {
            writer,

            dirs,
            files,

            written_files: BTreeMap::new(),
        }
    }

    /// Sorts the files by their depth in the directory tree
    /// Files with higher depth are written first
    fn sort_by_depth(files: &mut Vec<file::File>) {
        files.sort_by(|a, b| {
            let a_depth = a.path.split('/').count();
            let b_depth = b.path.split('/').count();
            a_depth.cmp(&b_depth)
        });
    }

    /// Writes the file data, directory data, and the path table to the given writer, returning a
    /// tuple containing the root directory and the path table.
    pub fn write(&mut self) -> Result<(DirectoryRef, DirectoryRef), std::io::Error> {
        self.write_file_data()?;
        let root_dir = self.write_directory_data()?;
        let path_table = self.write_path_table(&root_dir)?;
        Ok((root_dir, path_table))
    }

    fn write_file_data(&mut self) -> Result<(), std::io::Error> {
        log::trace!("Started writing file data");
        for file in &self.files {
            let data = file.data.get_data();
            //let size_aligned = (data.len() + 2047) & !2047;
            self.written_files.insert(
                file.path.clone(),
                (
                    false,
                    DirectoryRef {
                        offset: IsoImage::current_sector(self.writer) as u64,
                        size: data.len() as u64,
                    },
                ),
            );
            self.writer.write_all(&data)?;
            IsoImage::align(self.writer)?;
        }
        Ok(())
    }

    fn write_directory_data(&mut self) -> Result<DirectoryRef, std::io::Error> {
        log::trace!("Started writing directory data");
        let current_dir_ent =
            DirectoryRecord::new(&[0x00], DirectoryRef::default(), FileFlags::DIRECTORY);
        let parent_dir_ent =
            DirectoryRecord::new(&[0x01], DirectoryRef::default(), FileFlags::DIRECTORY);

        // In the first pass, we just write all of the directories from the leaves
        for file in &self.dirs {
            let start_sector = IsoImage::current_sector(self.writer);
            // We can just leave these as default, we modify them in a second pass
            current_dir_ent.write(self.writer)?;
            parent_dir_ent.write(self.writer)?;

            for entry in file.get_children() {
                let fullname = if file.path.is_empty() {
                    entry.to_string()
                } else {
                    format!("{}/{}", file.path, entry)
                };
                log::trace!("Processing directory record for {}", fullname);
                let stem = entry.split('/').last().unwrap_or(&entry);
                let (is_dir, file_ref) = self.written_files.get(&fullname).unwrap();
                let flags = if *is_dir {
                    FileFlags::DIRECTORY
                } else {
                    FileFlags::empty()
                };
                log::trace!("Writing directory record for {}", fullname);
                DirectoryRecord::new(stem.as_bytes(), *file_ref, flags).write(self.writer)?;
            }

            let end = IsoImage::align(self.writer)?;
            let directory_ref = DirectoryRef {
                offset: start_sector as u64,
                size: end - start_sector as u64 * 2048,
            };
            self.written_files
                .insert(file.path.clone(), (true, directory_ref));
        }

        let root_dir = self.written_files.get("").unwrap().clone();
        let mut stack = vec![(root_dir.1, root_dir.1, "".to_string())];

        while let Some((dir_ref, parent_ref, cur_path)) = stack.pop() {
            let start = dir_ref.offset * 2048;
            self.writer.seek(SeekFrom::Start(start))?;

            DirectoryRecord::new(&[0x00], dir_ref, FileFlags::DIRECTORY).write(&mut self.writer)?;
            DirectoryRecord::new(&[0x01], parent_ref, FileFlags::DIRECTORY)
                .write(&mut self.writer)?;

            let mut reader = IsoDirectory {
                reader: self.writer,
                directory: dir_ref,
            };
            for (offset, directory) in reader
                .entries()?
                .iter()
                .filter(|(_offset, entry)| entry.header.is_directory())
            {
                // Special cases for the current and parent directories
                if directory.name.bytes() == b"\x00" || directory.name.bytes() == b"\x01" {
                    continue;
                }
                let dirname = format!("{}/{}", cur_path, directory.name);
                let dir_ref_inner = self.written_files.get(dirname.as_str()).unwrap().1;
                let mut new_entry = directory.clone();
                new_entry.header.extent.write(dir_ref_inner.offset as u32);
                new_entry.header.data_len.write(dir_ref_inner.size as u32);
                self.writer.seek(SeekFrom::Start(start + offset))?;
                new_entry.write(&mut self.writer)?;
                stack.push((dir_ref_inner, dir_ref, dirname));
            }
        }

        // We need to seek back to the end of the directory record list, which is the root directory
        self.writer
            .seek(SeekFrom::Start(root_dir.1.offset * 2048 + root_dir.1.size))?;

        Ok(root_dir.1)
    }

    fn write_path_table(
        &mut self,
        root_dir: &DirectoryRef,
    ) -> Result<DirectoryRef, std::io::Error> {
        log::trace!("Started writing path table");
        let start_sector = IsoImage::current_sector(self.writer);
        let mut entries = Vec::new();
        let mut index = 1; // Root directory is always index 1
        let mut parent_map = std::collections::HashMap::new();

        // Write the root directory
        entries.push(PathTableEntry {
            length: 1,
            extended_attr_record: 0,
            parent_lba: root_dir.offset as u32,
            parent_index: 1,
            name: "\0".to_string(),
        });

        parent_map.insert("".to_string(), 1);

        for file in &self.dirs {
            if file.path.is_empty() {
                // We already wrote the root directory
                continue;
            }
            let (_, directory_ref) = self.written_files.get(&file.path).unwrap();
            let parent_name = file.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");

            let parent_index = *parent_map.get(parent_name).unwrap_or(&1);
            parent_map.insert(file.path.clone(), index);
            let name = file
                .path
                .rsplit_once('/')
                .map(|(_, n)| n)
                .unwrap_or(&file.path);

            entries.push(PathTableEntry {
                length: name.len() as u8,
                name: name.to_string(),
                extended_attr_record: 0,
                parent_lba: directory_ref.offset as u32,
                parent_index,
            });

            index += 1;
        }

        // Write L-Table (Little-Endian)
        for entry in &entries {
            self.writer
                .write_all(&entry.to_bytes(types::EndianType::LittleEndian))?;
        }

        // Align to sector boundary
        let end = IsoImage::align(self.writer)?;

        // We only store the L-table ref, but the M-table can be found by just adding the size of
        // the L-table to the offset of the L-table to find the offset of the M-table.
        let path_table_ref = DirectoryRef {
            offset: start_sector as u64,
            size: end - start_sector as u64 * 2048,
        };

        // Write M-Table (Big-Endian)
        for entry in &entries {
            self.writer
                .write_all(&entry.to_bytes(types::EndianType::BigEndian))?;
        }

        let mtable_end = IsoImage::align(self.writer)?;
        assert_eq!(mtable_end - end, path_table_ref.size);

        Ok(path_table_ref)
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct ProtectiveMBR {
    boot_code: [u8; 446],      // Empty or boot code
    partition_entry: [u8; 16], // Protective Partition
    reserved: [u8; 48],        // Unused
    boot_signature: [u8; 2],   // Must be [0x55, 0xAA]
}

unsafe impl bytemuck::Zeroable for ProtectiveMBR {}
unsafe impl bytemuck::Pod for ProtectiveMBR {}

impl ProtectiveMBR {
    pub fn new(total_sectors: u32) -> Self {
        ProtectiveMBR {
            boot_code: [0; 446],
            partition_entry: [
                0x00,
                0xFF,
                0xFF,
                0xFF, // Status & CHS Start
                0x17, // Partition Type (Hidden NTFS / ISO)
                0xFF,
                0xFF,
                0xFF, // CHS End
                0x01,
                0x00,
                0x00,
                0x00, // LBA Start (1 sector after MBR)
                (total_sectors & 0xFF) as u8,
                ((total_sectors >> 8) & 0xFF) as u8,
                ((total_sectors >> 16) & 0xFF) as u8,
                ((total_sectors >> 24) & 0xFF) as u8,
            ],
            reserved: [0; 48],
            boot_signature: [0x55, 0xAA],
        }
    }
}

#[cfg(all(test, feature = "el-torito"))]
mod tests {}

#[cfg(all(test, not(feature = "el-torito")))]
mod tests {}

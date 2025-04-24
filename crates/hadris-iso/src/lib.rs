//! Hadris ISO
//! Terminology and spec are followed by the specifications described in
//! the [non official ISO9660 specification included](https://github.com/hxyulin/hadris/tree/main/crates/hadris-iso/spec)

#[cfg(feature = "el-torito")]
pub mod boot;
#[cfg(feature = "el-torito")]
pub use boot::*;

use bytemuck::Zeroable;
use hadris_common::part::{
    gpt::{GptPartitionEntry, GptPartitionTableHeader, Guid},
    mbr::{Chs, MbrPartitionTable, MbrPartitionType},
};

pub use directory::*;
pub use file::*;
pub use options::*;
pub use path::*;
// We expose these types because they are used in the public API,
// but they are also just std::io types of hadris-io types (if in no-std mode)
pub use hadris_io::{Error, Read, Seek, SeekFrom, Write};

extern crate alloc;

use alloc::collections::BTreeMap;
use core::fmt::Debug;
pub use types::*;
pub use volume::*;

mod directory;
mod file;
mod options;
mod path;
mod types;
mod volume;

/// Errors that can occur when working with an ISO image
#[derive(Debug, thiserror::Error)]
pub enum IsoImageError {
    #[cfg(feature = "extra-checks")]
    /// The image is too small, check [`FormatOptions::image_len()`] for the minimum size
    #[error("The image is too small, expected at least {0}b, got {1}b")]
    ImageTooSmall(u64, u64),

    /// An IO error occurred
    ///
    /// When working with the `std` feature, this is an alias for [`std::io::Error`]
    /// When working with the `no-std` feature, this is an alias for [`hadris_io::Error`]
    #[error(transparent)]
    IoError(#[from] hadris_io::Error),
}

/// An ISO image
///
/// This is the main struct for working with ISO images.
///
/// # Example
/// To create a new ISO image, you can use the [`Self::format_file`] method. \
/// This example creates a hybrid bootable image with a BIOS boot entry and a UEFI boot entry:
/// ```
/// use hadris_iso::{IsoImage, FormatOptions, FileInput, FileInterchange, BootOptions, BootEntryOptions, EmulationType, PlatformId, BootSectionOptions};
/// use std::path::PathBuf;
///
/// let files = PathBuf::from("path/to/iso_root");
/// # // Now we need to actually create a temporary directory
/// # let files = tempfile::tempdir()?.into_path();
/// # let mut tmpfile = std::fs::File::create(files.join("boot.img"))?;
/// # use std::io::Write;
/// # writeln!(tmpfile, "Hello, world!")?;
/// # drop(tmpfile);
/// # let mut tmpfile = std::fs::File::create(files.join("uefi-boot.img"))?;
/// # writeln!(tmpfile, "Hello, world!")?;
/// # drop(tmpfile);
/// let options = FormatOptions::new()
/// .with_files(FileInput::from_fs(&files)?)
/// .with_level(FileInterchange::NonConformant)
/// .with_boot_options(BootOptions {
///     write_boot_catalogue: true,
///     default: BootEntryOptions {
///         boot_image_path: "boot.img".to_string(),
///         load_size: 4,
///         emulation: EmulationType::NoEmulation,
///         boot_info_table: true,
///         grub2_boot_info: false,
///     },
///     entries: vec![(
///         BootSectionOptions {
///             platform_id: PlatformId::UEFI,
///         },
///         BootEntryOptions {
///             boot_image_path: "uefi-boot.img".to_string(),
///             load_size: 0, // This means the size will be calculated
///             emulation: EmulationType::NoEmulation,
///             boot_info_table: false,
///             grub2_boot_info: false,
///         },
///     )],
/// });
/// let output_file = PathBuf::from("my_image.iso");
/// # let output_file = files.join("my_image.iso");
/// let file = IsoImage::format_file(output_file, options)?;
/// # Ok::<(), hadris_iso::IsoImageError>(())
/// ````
#[derive(Debug)]
pub struct IsoImage<'a, T: Read + Write + Seek> {
    data: &'a mut T,

    volume_descriptors: VolumeDescriptorList,
    root_directory: DirectoryRef,
    path_table: PathTableRef,
}

impl<'a> IsoImage<'a, std::fs::File> {
    /// Formats a new ISO image,
    ///
    /// This creates a new file, which may be too large for some cases,
    /// but it will be truncated to the correct size when the image is written.
    /// This may only be an issue when low on disk space or using an in-memory filesystem. 
    /// Due to how many operating systems work with files, the pages should be mapped-on-demand,
    /// and there shouldn't be a lot of performance penalty.
    pub fn format_file<P>(path: P, options: FormatOption) -> Result<std::fs::File, IsoImageError>
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
        IsoImage::format_new(&mut file, options)?;
        let written = file.stream_position()?;
        log::debug!("Written {written}b to image, trimming...");
        file.set_len(written)?;
        file.flush()?;
        Ok(file)
    }
}

impl<'a, T: Read + Write + Seek> IsoImage<'a, T> {
    /// Formats a new ISO image,
    /// for a more convenient API, see [`Self::format_file`] for [`std::fs::File`]
    /// Otherwise, resize the image using the minimum / maximum from [`FormatOptions::image_len`].
    pub fn format_new(data: &'a mut T, mut ops: FormatOption) -> Result<(), IsoImageError> {
        #[cfg(feature = "extra-checks")]
        if ops.strictness >= Strictness::Default {
            let size_bytes = data.seek(SeekFrom::End(0))?;
            let (min_size, _max_size) = ops.image_len();
            if size_bytes < min_size {
                return Err(IsoImageError::ImageTooSmall(min_size, size_bytes));
            }

            log::trace!(
                "Started formatting ISO image with {} sectors ({}) bytes)",
                size_bytes / 2048,
                size_bytes
            );
        }

        let mut volume_descriptors = VolumeDescriptorList::empty();

        volume_descriptors.push(VolumeDescriptor::Primary(PrimaryVolumeDescriptor::new(
            ops.volume_name.as_str(),
            0, // We populate the size later
        )));

        // Add El-Torito catalog file and volume descriptor
        #[cfg(feature = "el-torito")]
        if let Some(boot_ops) = &ops.boot {
            let boot_record = boot::ElToritoWriter::create_descriptor(boot_ops, &mut ops.files);
            volume_descriptors.push(VolumeDescriptor::BootRecord(boot_record));
        }

        let mut current_index: u64 = 16 * 2048;
        // We don't need to write it yet, since we have to write it later anyways
        current_index += volume_descriptors.size_required() as u64;
        data.seek(SeekFrom::Start(current_index as u64))?;
        // Current Pos: After volume descriptors

        let mut file_writer = FileWriter::new(data, ops.level, ops.files);
        let (root_dir, path_table) = file_writer.write()?;
        // Current Pos: After file data + directory records

        {
            log::trace!("Updating primary volume descriptor");
            let pvd = volume_descriptors.primary_mut();
            pvd.dir_record.header.extent.write(root_dir.offset as u32);
            pvd.dir_record.header.data_len.write(root_dir.size as u32);
            pvd.path_table_size.write(path_table.size as u32);
            pvd.type_l_path_table.set(path_table.offset as u32);
            pvd.type_m_path_table
                .set(path_table.offset as u32 + (path_table.size / 2048) as u32);
            // Current Pos: After Path Tables
        }

        #[cfg(feature = "el-torito")]
        if let Some(boot_ops) = ops.boot {
            // TODO: If we support nested files, we need to find them from the Path table, and not
            // the root directory

            // TODO: Support more than just the default entry
            let mut catalog = BootCatalogue::default();

            let current_index = Self::align(data)?;

            for (section, mut entry) in boot_ops.sections() {
                // TODO: We need to abstract this, because this only allows searching root directory
                let name = ops.level.from_str(&entry.boot_image_path).unwrap();
                let (_, file) = IsoDir {
                    reader: data,
                    directory: root_dir.clone(),
                }
                .entries()?
                .iter()
                .find(|(_idx, e)| e.name == name)
                .unwrap()
                .clone();

                if entry.load_size == 0 {
                    entry.load_size = ((file.header.data_len.read() + 511) / 512) as u16;
                }
                let boot_image_lba = file.header.extent.read();
                let boot_entry =
                    BootSectionEntry::new(entry.emulation, 0, entry.load_size, boot_image_lba);

                if let Some(section) = section {
                    catalog.add_section(section.platform_id, vec![boot_entry]);
                } else {
                    // If it is the default entry, it doesn't have a section
                    catalog.set_default_entry(boot_entry);
                }

                if entry.boot_info_table {
                    let mut checksum = 0u32;
                    let mut buffer = [0u8; 4];
                    data.seek(SeekFrom::Start(
                        file.header.extent.read() as u64 * 2048 + 64,
                    ))?;
                    for _ in (64..file.header.data_len.read()).step_by(4) {
                        // PERF: We might be able to use simd loading and operations here?
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
            }

            let name = ops.level.from_str("boot.catalog").unwrap();
            log::trace!("Searching for boot catalogue ({})", name);
            let catalog_ptr = if boot_ops.write_boot_catalogue {
                let (_, catalog_file) = IsoDir {
                    reader: data,
                    directory: root_dir.clone(),
                }
                .entries()?
                .iter()
                .find(|(_idx, e)| e.name == name)
                .expect("Could not find the boot catalogue in ISO filesystem")
                .clone();

                let catalog_start = catalog_file.header.extent.read() as u64;
                data.seek(SeekFrom::Start(catalog_start * 2048))?;
                assert!(catalog_file.header.data_len.read() as usize >= catalog.size());
                catalog.write(data)?;
                data.seek(SeekFrom::Start(current_index))?;

                catalog_start as u32
            } else {
                data.seek(SeekFrom::Start(current_index))?;
                catalog.write(data)?;
                (Self::align(data)? / 2048) as u32
            };

            volume_descriptors
                .boot_record_mut()
                .unwrap()
                .catalog_ptr
                .set(catalog_ptr);
            Self::align(data)?;
        }

        if let Some(system_area) = &ops.system_area {
            let restore_pos = data.stream_position()?;
            assert!(system_area.len() <= 16 * 2048);
            data.seek(SeekFrom::Start(0))?;
            data.write_all(system_area)?;
            data.seek(SeekFrom::Start(restore_pos))?;
        }

        let write_format =
            ops.system_area.is_none() || ops.format.contains(PartitionOptions::OVERWRITE_FORMAT);

        if write_format && ops.format.contains(PartitionOptions::GPT) {
            log::trace!("Writing Guid Partition Table at 512b");
            // Current sector in terms of 512 byte sectors
            let current_sector = data.stream_position()? / 512;

            data.seek(SeekFrom::Start(512))?;
            let mut buf: [u8; 512] = [0; 512];
            data.read_exact(&mut buf)?;
            data.seek(SeekFrom::Start(512))?;
            for i in 0..512 {
                if buf[i] != 0 {
                    log::warn!(
                        "Found non-zero byte at offset {}, this will be overwritten by the GPT",
                        i + 512
                    );
                }
            }
            let mut gpt = GptPartitionTableHeader::default();

            // We need to be careful here, because the LBA is 512 bytes, but we are using 2048 byte sectors

            gpt.current_lba.set(1);
            let sectors_used_by_entries = 128 * 128 / 512;
            let backup_sector = current_sector + sectors_used_by_entries;
            assert!(
                sectors_used_by_entries + 1 < 64,
                "GPT partition overrides volume descriptors"
            );
            // The first non 'system area' / usable sector is at 16 * 2048b
            gpt.first_usable_lba.set(64);
            gpt.partition_entry_lba.set(2);
            // Subtract 1 because GPT ranges are inclusive
            gpt.last_usable_lba.set(current_sector - 1);
            gpt.backup_lba.set(backup_sector);
            gpt.disk_guid = Guid::generate_v4();
            gpt.num_partition_entries.set(128);

            let entries = [GptPartitionEntry::zeroed(); 128];
            use hadris_common::alg::hash::crc::Crc32HasherIsoHdlc;
            let checksum = Crc32HasherIsoHdlc::checksum(bytemuck::bytes_of(&entries));
            gpt.partition_entry_array_crc32.set(checksum);

            // We need to extend image size to include the backup entries and header
            gpt.generate_crc32();

            // Write primary GPT
            data.write_all(bytemuck::bytes_of(&gpt))?;
            data.write_all(bytemuck::bytes_of(&entries))?;

            // Write backup GPT
            data.seek(SeekFrom::Start(current_sector * 512))?;
            gpt.partition_entry_lba
                .set(backup_sector - sectors_used_by_entries);
            gpt.generate_crc32();
            data.write_all(bytemuck::bytes_of(&entries))?;
            data.write_all(bytemuck::bytes_of(&gpt))?;

            // GPT is aligned to 512 bytes, but we need 2048 bytes aligned
            IsoImage::align(data)?;
        }

        let size_bytes = data.stream_position()?;
        let size_sectors = size_bytes / 2048;

        if write_format && ops.format.contains(PartitionOptions::INCLUDE_DEFAULT_BOOT) {
            data.seek(SeekFrom::Start(0))?;
            assert_eq!(hadris_common::BOOT_SECTOR_BIN.len(), 512);
            data.write_all(hadris_common::BOOT_SECTOR_BIN)?;
        }

        if write_format && ops.format.contains(PartitionOptions::MBR) {
            if ops.strictness >= Strictness::Default {
                data.seek(SeekFrom::Start(446))?;
                let mut buf: [u8; 66] = [0; 66];
                data.read_exact(&mut buf)?;
                for i in 0..64 {
                    if buf[i] != 0 {
                        log::warn!(
                            "Found non-zero byte at offset {}, this will be overwritten by the MBR",
                            i + 446
                        );
                    }
                }

                if buf[64] != 0x55 || buf[65] != 0xAA && (buf[64] != 0 && buf[65] != 0) {
                    log::warn!(
                        "Expected boot signature 0x55AA at offset 64, got 0x{:x}{:x}",
                        buf[64],
                        buf[65]
                    );
                }
            }
            // TODO: Maybe an option for the user to include the generic boot sector stub?
            data.seek(SeekFrom::Start(446))?;
            let mut mbr = MbrPartitionTable::default();
            let block_count = u32::try_from(size_sectors * 4).unwrap_or(u32::MAX);

            mbr.partitions[0].start_head = Chs::new(1);
            mbr.partitions[0].end_head = Chs::new(block_count);
            let part_type = if ops.format.contains(PartitionOptions::PROTECTIVE_MBR) {
                log::trace!("Using protective MBR");
                MbrPartitionType::ProtectiveMbr
            } else {
                log::trace!("Using ISO9660 MBR");
                MbrPartitionType::Iso9660
            };

            mbr.partitions[0].part_type = part_type.to_u8();
            mbr.partitions[0].start_sector.set(1);
            mbr.partitions[0].block_count.set(block_count);

            data.write_all(bytemuck::bytes_of(&mbr))?;
            data.write_all(&[0x55, 0xAA])?;
        }

        data.seek(SeekFrom::Start(16 * 2048))?;
        volume_descriptors
            .primary_mut()
            .volume_space_size
            .write((size_bytes / 2048) as u32);
        volume_descriptors.write(data)?;

        // We need to be at the end of the image
        data.seek(SeekFrom::Start(size_bytes))?;
        Ok(())
    }

    #[deprecated(since = "0.0.1", note = "Use `parse` instead")]
    pub fn new(data: &'a mut T) -> Result<Self, Error> {
        Self::parse(data)
    }

    /// Parses an ISO image from the given reader
    /// Currently this is not fully supported, and only provides basic information
    pub fn parse(data: &'a mut T) -> Result<Self, Error> {
        {
            data.seek(SeekFrom::Start(446))?;
            let mut mbr = MbrPartitionTable::default();
            data.read_exact(bytemuck::bytes_of_mut(&mut mbr))?;
            if mbr.is_valid() {
                let len = mbr.len();
                log::trace!("Found MBR partition table with {} entries", len);
                for i in 0..len {
                    log::trace!("\tPartition {}:", i);
                    log::trace!("\t\tStart sector: {}", mbr[i].start_sector);
                    log::trace!("\t\tSector count: {}", mbr[i].block_count);
                    log::trace!(
                        "\t\tType: {:?}",
                        MbrPartitionType::from_u8(mbr[i].part_type)
                    );
                }
            }
        }

        {
            data.seek(SeekFrom::Start(512))?;
            let mut gpt_header = GptPartitionTableHeader::default();
            data.read_exact(bytemuck::bytes_of_mut(&mut gpt_header))?;
            if gpt_header.is_valid() {
                log::trace!(
                    "Found GPT partition table with {} entries",
                    gpt_header.num_partition_entries
                );
                let checksum = gpt_header.crc32.get();
                gpt_header.generate_crc32();
                if checksum != gpt_header.crc32.get() {
                    log::warn!(
                        "GPT header CRC32 checksum mismatch, got {:#x}, expected {:#x}",
                        gpt_header.crc32.get(),
                        checksum
                    );
                }
                log::trace!("\tRevision: {}", gpt_header.revision);
                log::trace!("\tHeader size: {}", gpt_header.header_size);
                log::trace!("\tCRC32: {}", gpt_header.crc32);
                log::trace!("\tDisk GUID: {}", gpt_header.disk_guid);
                log::trace!("\tCurrent LBA: {}", gpt_header.current_lba);
                log::trace!("\tBackup LBA: {}", gpt_header.backup_lba);
                log::trace!("\tFirst usable LBA: {}", gpt_header.first_usable_lba);
                log::trace!("\tLast usable LBA: {}", gpt_header.last_usable_lba);
                log::trace!("\tPartition entry LBA: {}", gpt_header.partition_entry_lba);
                log::trace!(
                    "\tNum partition entries: {}",
                    gpt_header.num_partition_entries
                );
                log::trace!(
                    "\tSize of partition entry: {}",
                    gpt_header.size_of_partition_entry
                );
                log::trace!(
                    "\tPartition entry array CRC32: {}",
                    gpt_header.partition_entry_array_crc32
                );

                data.seek(SeekFrom::Start(
                    gpt_header.partition_entry_lba.get() as u64 * 512,
                ))?;
                let mut entries = vec![
                    GptPartitionEntry::zeroed();
                    gpt_header.num_partition_entries.get() as usize
                ];
                data.read_exact(bytemuck::cast_slice_mut(&mut entries))?;
                for entry in entries.iter_mut() {
                    if entry.is_empty() {
                        continue;
                    }
                    let name = entry
                        .partition_name
                        .to_string()
                        .unwrap_or("Invalid UTF-8".to_string());
                    log::trace!("\tPartition {}:", name);
                    log::trace!("\t\tType GUID: {}", entry.type_guid);
                    log::trace!("\t\tUnique GUID: {}", entry.unique_partition_guid);
                    log::trace!("\t\tStarting LBA: {}", entry.starting_lba);
                    log::trace!("\t\tEnding LBA: {}", entry.ending_lba);
                    log::trace!("\t\tAttributes: {}", entry.attributes);
                    log::trace!("\t\tPartition name: {}", name);
                }

                let backup = gpt_header.backup_lba.get() as u64 * 512;
                data.seek(SeekFrom::Start(backup))?;
                let mut backup_header = GptPartitionTableHeader::default();
                data.read_exact(bytemuck::bytes_of_mut(&mut backup_header))?;
                if !backup_header.is_valid() {
                    log::warn!("Found invalid backup GPT header at LBA {}", backup);
                }
                // TODO: Calculate the checksum for backup
            }
        }

        data.seek(SeekFrom::Start(16 * 2048))?;
        let volume_descriptors = VolumeDescriptorList::parse(data)?;

        let pvd = volume_descriptors.primary();
        #[cfg(feature = "el-torito")]
        if let Some(boot) = volume_descriptors.boot_record() {
            data.seek(SeekFrom::Start(boot.catalog_ptr.get() as u64 * 2048))?;
            let catalogue = BootCatalogue::parse(data)?;
            log::trace!("Boot catalogue: {:?}", catalogue);
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

    pub fn root_directory(&mut self) -> IsoDir<T> {
        IsoDir {
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
        let seek = data.seek(SeekFrom::Current(0)).unwrap();
        assert!(seek % 2048 == 0, "Seek must be a multiple of 2048");
        (seek / 2048) as usize
    }

    fn align(data: &mut T) -> Result<u64, Error> {
        let current_seek = data.stream_position()?;
        let padded_end = (current_seek + 2047) & !2047;
        data.seek(SeekFrom::Start(padded_end))?;
        Ok(padded_end)
    }
}

#[derive(Debug)]
struct FileWriter<'a, W: Read + Write + Seek> {
    writer: &'a mut W,

    level: FileInterchange,
    dirs: Vec<file::File>,
    files: Vec<file::File>,

    /// The first element is whether the file is a directory
    written_files: BTreeMap<String, (bool, DirectoryRef)>,
}

impl<'a, W: Read + Write + Seek> FileWriter<'a, W> {
    pub fn new(writer: &'a mut W, level: FileInterchange, files: FileInput) -> Self {
        log::trace!("Started writing files");
        let (mut dirs, files) = files.split();

        log::trace!("Sorting directories by depth");
        Self::sort_by_depth(&mut dirs);

        Self {
            writer,

            level,
            dirs,
            files,

            written_files: BTreeMap::new(),
        }
    }

    /// Sorts the files by their depth in the directory tree
    /// Files with higher depth are written first
    fn sort_by_depth(files: &mut Vec<file::File>) {
        files.sort_by(|a, b| {
            // PERF: We can probably pre-compute the depths here when generating the FileInput
            let a_depth = a.path.split('/').count();
            let b_depth = b.path.split('/').count();
            if a_depth == b_depth {
                return b.path.len().cmp(&a.path.len());
            }
            b_depth.cmp(&a_depth)
        });
    }

    /// Writes the file data, directory data, and the path table to the given writer, returning a
    /// tuple containing the root directory and the path table.
    pub fn write(&mut self) -> Result<(DirectoryRef, DirectoryRef), Error> {
        self.write_file_data()?;
        let root_dir = self.write_directory_data()?;
        let path_table = self.write_path_table(&root_dir)?;
        Ok((root_dir, path_table))
    }

    fn write_file_data(&mut self) -> Result<(), Error> {
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

    fn write_directory_data(&mut self) -> Result<DirectoryRef, Error> {
        log::trace!("Started writing directory data");
        let default_entry = DirectoryRecord::with_len(1);

        // In the first pass, we just write all of the directories from the leaves
        for file in &self.dirs {
            let start_sector = IsoImage::current_sector(self.writer);
            // We can just leave these as default, we modify them in a second pass
            default_entry.write(self.writer)?;
            default_entry.write(self.writer)?;

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
                let name = self.level.from_str(stem).unwrap();
                DirectoryRecord::new(name, *file_ref, flags).write(self.writer)?;
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

            DirectoryRecord::new(
                IsoStringFile::from_bytes(&[0x00]),
                dir_ref,
                FileFlags::DIRECTORY,
            )
            .write(self.writer)?;
            DirectoryRecord::new(
                IsoStringFile::from_bytes(&[0x01]),
                parent_ref,
                FileFlags::DIRECTORY,
            )
            .write(self.writer)?;

            let mut reader = IsoDir {
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
                let orig_name = self.level.original(&directory.name);
                let dirname = if cur_path.is_empty() {
                    orig_name
                } else {
                    format!("{}/{}", cur_path, orig_name)
                };
                let dir_ref_inner = self.written_files.get(dirname.as_str()).unwrap().1;
                let mut new_entry = directory.clone();
                assert_eq!(new_entry.name, directory.name, "Directory name mismatch");
                new_entry.header.extent.write(dir_ref_inner.offset as u32);
                new_entry.header.data_len.write(dir_ref_inner.size as u32);
                self.writer.seek(SeekFrom::Start(start + offset))?;

                new_entry.write(self.writer)?;
                stack.push((dir_ref_inner, dir_ref, dirname));
            }
        }

        // We need to seek back to the end of the directory record list, which is the root directory
        self.writer
            .seek(SeekFrom::Start(root_dir.1.offset * 2048 + root_dir.1.size))?;

        Ok(root_dir.1)
    }

    fn write_path_table(&mut self, root_dir: &DirectoryRef) -> Result<DirectoryRef, Error> {
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
                .write_all(&entry.to_bytes(EndianType::LittleEndian))?;
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
                .write_all(&entry.to_bytes(EndianType::BigEndian))?;
        }

        let mtable_end = IsoImage::align(self.writer)?;
        // This is just a sanity check
        debug_assert_eq!(mtable_end - end, path_table_ref.size);

        Ok(path_table_ref)
    }
}

/// Trait for internal methods of the `IsoImage` struct.
///
/// This trait provides a way to access some of the internal structures of the `IsoImage` struct,
/// and not only the public API (files, boot entries, etc.).
pub trait VolumeInternals {
    /// Returns a reference to the volume descriptors.
    fn get_volume_descriptors(&self) -> &[VolumeDescriptor];
}

impl<'a, T: Read + Write + Seek> VolumeInternals for IsoImage<'a, T> {
    fn get_volume_descriptors(&self) -> &[VolumeDescriptor] {
        self.volume_descriptors.descriptors.as_slice()
    }
}

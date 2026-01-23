use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt;

mod writer;

use crate::{
    boot::{BootCatalog, BootInfoTable, BootSectionEntry, ElToritoWriter},
    directory::{DirectoryRecord, DirectoryRef, FileFlags},
    file::EntryType,
    io::{IsoCursor, LogicalSector},
    joliet::JolietLevel,
    path::{PathTableEntryHeader, PathTableRef},
    read::PathSeparator,
    volume::{
        BootRecordVolumeDescriptor, PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor,
        VolumeDescriptor, VolumeDescriptorHeader, VolumeDescriptorList, VolumeDescriptorType,
    },
    write::writer::{PathTableWriter, WrittenDirectory, WrittenFile, WrittenFiles},
};
use hadris_common::types::{
    endian::{Endian, EndianType},
    number::U32,
};
use hadris_part::{
    gpt::{Guid, GptPartitionEntry},
    hybrid::HybridMbrBuilder,
    mbr::{Chs, MasterBootRecord, MbrPartition, MbrPartitionType},
};
use options::PartitionScheme;
use hadris_io::{self as io, Read, Seek, SeekFrom, Write};

use alloc::{collections::VecDeque, string::String, vec, vec::Vec};

pub mod options;
use options::FormatOptions;

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
            children_files.push(File::File {
                name: Arc::new(name),
                contents,
            });
        } else if path.is_dir() {
            let grand_children = read_directory_recursively(&path)?;
            children_files.push(File::Directory {
                name: Arc::new(name),
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
    File {
        name: Arc<String>,
        contents: Vec<u8>,
    },
    Directory {
        name: Arc<String>,
        children: Vec<File>,
    },
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
    pub fn name(&self) -> Arc<String> {
        match self {
            File::File { name, .. } => name.clone(),
            File::Directory { name, .. } => name.clone(),
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
    data: IsoCursor<DATA>,
    entry_types: Vec<EntryType>,
    ops: FormatOptions,
    written_files: WrittenFiles,
    path_tables: BTreeMap<EntryType, PathTableRef>,
}

impl<DATA: Read + Write + Seek> IsoImageWriter<DATA> {
    pub fn format_new(
        data: DATA,
        mut files: InputFiles,
        ops: FormatOptions,
    ) -> IsoCreationResult<()> {
        let mut writer = Self::new(data, ops);
        writer.write_volume_descriptors(&mut files)?;
        let root_dirs = writer.write_files(&files)?;
        writer.write_path_tables()?;
        writer.finalize_volume_descriptors(root_dirs)?;
        Ok(())
    }

    fn new(data: DATA, ops: FormatOptions) -> Self {
        let mut entry_types = Vec::new();
        entry_types.push(ops.features.filenames.into());
        if ops.features.long_filenames {
            entry_types.push(EntryType::Level3 {
                supports_lowercase: true,
                supports_rrip: false,
            });
        }
        if let Some(joliet) = ops.features.joliet {
            entry_types.push(joliet.into());
        }

        Self {
            data: IsoCursor::new(data, ops.sector_size),
            ops,
            entry_types,
            written_files: WrittenFiles::new(),
            path_tables: BTreeMap::new(),
        }
    }

    const VOLUME_DESCRIPTOR_SET_START: LogicalSector = LogicalSector(16);

    fn write_volume_descriptors(&mut self, files: &mut InputFiles) -> io::Result<()> {
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START)?;
        let mut volume_descriptors = VolumeDescriptorList::empty();
        for &entry in &self.entry_types {
            match entry {
                EntryType::Level1 { .. } | EntryType::Level2 { .. } => {
                    let mut pvd = PrimaryVolumeDescriptor::new(&self.ops.volume_name, 0);
                    pvd.dir_record.header.len = 34;
                    pvd.volume_sequence_number.write(1);
                    volume_descriptors.push(VolumeDescriptor::Primary(pvd));
                }
                EntryType::Level3 { .. } => {
                    // Version 2 for EVD
                    let mut evd = SupplementaryVolumeDescriptor::new_evd(&self.ops.volume_name, 0);
                    evd.dir_record.header.len = 34;
                    evd.volume_sequence_number.write(1);
                    volume_descriptors.push(VolumeDescriptor::Supplementary(evd));
                }
                EntryType::Joliet { level, .. } => {
                    let mut svd = SupplementaryVolumeDescriptor::new_svd(
                        &self.ops.volume_name,
                        0,
                        level.escape_sequence(),
                    );
                    svd.dir_record.header.len = 34;
                    svd.volume_sequence_number.write(1);
                    volume_descriptors.push(VolumeDescriptor::Supplementary(svd));
                }
            }
        }

        if let Some(boot) = &self.ops.features.el_torito {
            let boot_record = ElToritoWriter::create_descriptor(boot, files);
            volume_descriptors.insert(1, VolumeDescriptor::BootRecord(boot_record));
        }

        volume_descriptors.write(&mut self.data)?;
        Ok(())
    }

    fn finalize_volume_descriptors(
        &mut self,
        root_dirs: BTreeMap<EntryType, DirectoryRef>,
    ) -> io::Result<()> {
        // Write boot catalog
        let catalog_ptr = if let Some(boot) = &self.ops.features.el_torito {
            let mut catalog = BootCatalog::default();
            let current_sector = self.data.pad_align_sector()?;

            for (section, entry) in boot.sections() {
                let dir_ref = self
                    .written_files
                    .find_file(&entry.boot_image_path, self.ops.path_seperator)
                    .expect("failed to find boot image file");
                let load_size = entry
                    .load_size
                    .map(core::num::NonZeroU16::get)
                    .unwrap_or_else(|| ((dir_ref.size + 511) / 512) as u16);
                let boot_image_lba = dir_ref.extent.0 as u32;
                let boot_entry =
                    BootSectionEntry::new(entry.emulation, 0, load_size, boot_image_lba);
                if let Some(section) = section {
                    // TODO: Create Virtual FAT
                    catalog.add_section(section.platform, vec![boot_entry]);
                } else {
                    catalog.set_default_entry(boot_entry);
                }

                if entry.boot_info_table {
                    // Boot info table requires at least 64 bytes in the boot image
                    // (header is at offset 8-56, checksum covers bytes 64+)
                    if dir_ref.size < 64 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "boot image too small for boot info table (minimum 64 bytes)",
                        ));
                    }

                    let mut checksum = 0u32;
                    let mut buffer = [0u8; 4];
                    let byte_offset = (boot_image_lba as u64) * self.ops.sector_size as u64;
                    self.data.seek(SeekFrom::Start(byte_offset + 64))?;
                    // Calculate checksum for all 4-byte chunks from offset 64 to end
                    let checksum_bytes = dir_ref.size - 64;
                    for _ in 0..(checksum_bytes / 4) {
                        self.data.read_exact(&mut buffer)?;
                        checksum = checksum.wrapping_add(u32::from_le_bytes(buffer));
                    }
                    let table = BootInfoTable {
                        iso_start: U32::new(16),
                        file_lba: U32::new(dir_ref.extent.0 as u32),
                        file_len: U32::new(dir_ref.size as u32),
                        checksum: U32::new(checksum),
                    };

                    const TABLE_OFFSET: u64 = 8;
                    self.data
                        .seek(SeekFrom::Start(byte_offset + TABLE_OFFSET))?;
                    self.data.write_all(bytemuck::bytes_of(&table))?;
                }

                // TODO: Support GRUB2 Boot Info
            }

            if boot.write_boot_catalog {
                let dir_ref = self
                    .written_files
                    .find_file("boot.catalog", self.ops.path_seperator)
                    .expect("failed to find boot image file");
                self.data.seek_sector(dir_ref.extent)?;
                assert!(dir_ref.size >= catalog.size());
                catalog.write(&mut self.data)?;
                self.data.seek_sector(current_sector)?;

                Some(dir_ref.extent.0 as u32)
            } else {
                self.data.seek_sector(current_sector)?;
                catalog.write(&mut self.data)?;
                self.data.pad_align_sector()?;
                Some(current_sector.0 as u32)
            }
        } else {
            None
        };

        let end_sector = self.data.pad_align_sector()?;
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START)?;

        // TODO: How do we handle non-2048 byte sector sizes?
        let mut buffer = [0u8; 2048];
        loop {
            self.data.read_exact(&mut buffer)?;
            let header = VolumeDescriptorHeader::from_bytes(&buffer[0..7]);
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);
            if let VolumeDescriptorType::VolumeSetTerminator = ty {
                break;
            }
            assert!(header.is_valid());

            match ty {
                VolumeDescriptorType::PrimaryVolumeDescriptor => {
                    let base_type = self
                        .entry_types
                        .iter()
                        .find(|e| matches!(e, EntryType::Level1 { .. } | EntryType::Level2 { .. }))
                        .expect("failed to find base Level");
                    let root_dir = root_dirs.get(base_type).unwrap();
                    let pt = self.path_tables.get(base_type).unwrap();
                    let pvd = bytemuck::from_bytes_mut::<PrimaryVolumeDescriptor>(&mut buffer);
                    pvd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                    pvd.dir_record.header.data_len.write(root_dir.size as u32);
                    pvd.type_l_path_table.set(pt.lpt.0 as u32);
                    pvd.type_m_path_table.set(pt.mpt.0 as u32);
                    pvd.path_table_size.write(pt.size as u32);
                    pvd.volume_space_size.write(end_sector.0 as u32);
                }
                VolumeDescriptorType::SupplementaryVolumeDescriptor => {
                    let svd =
                        bytemuck::from_bytes_mut::<SupplementaryVolumeDescriptor>(&mut buffer);
                    match svd.header.version {
                        1 => {
                            for &level in JolietLevel::all() {
                                if svd.escape_sequences == level.escape_sequence() {
                                    let joliet = self
                                        .entry_types
                                        .iter()
                                        .find(
                                            |e| matches!(e, EntryType::Joliet{ level: jl, ..} if *jl == level),
                                        )
                                        .expect("joliet not found in entries!");
                                    let root_dir = root_dirs.get(joliet).unwrap();
                                    let pt = self.path_tables.get(joliet).unwrap();

                                    svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                                    svd.dir_record.header.data_len.write(root_dir.size as u32);
                                    svd.type_l_path_table.set(pt.lpt.0 as u32);
                                    svd.type_m_path_table.set(pt.mpt.0 as u32);
                                    svd.path_table_size.write(pt.size as u32);
                                    svd.volume_space_size.write(end_sector.0 as u32);
                                }
                            }
                        }
                        2 => {
                            if svd.escape_sequences != [b' '; 32] {
                                // We don't recognize this EVD
                                continue;
                            }

                            let l3 = self
                                .entry_types
                                .iter()
                                .find(|e| matches!(e, EntryType::Level3 { .. }))
                                .unwrap();
                            let root_dir = root_dirs.get(l3).unwrap();
                            svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                            svd.dir_record.header.data_len.write(root_dir.size as u32);
                            svd.volume_space_size.write(end_sector.0 as u32);
                        }

                        // Unknown version
                        _ => {}
                    }
                }
                VolumeDescriptorType::BootRecord => {
                    let catalog_ptr =
                        catalog_ptr.expect("image with boot record should have a catalog");
                    let boot_record =
                        bytemuck::from_bytes_mut::<BootRecordVolumeDescriptor>(&mut buffer);
                    boot_record.catalog_ptr.set(catalog_ptr);
                }
                // We don't do anything
                _ => continue,
            }

            // Write the new data
            self.data.seek_relative(-(buffer.len() as i64))?;
            self.data.write(&buffer)?;
        }

        // Now we finalize the partition tables based on hybrid boot options
        self.write_partition_tables(end_sector)?;

        Ok(())
    }

    fn write_files(&mut self, files: &InputFiles) -> io::Result<BTreeMap<EntryType, DirectoryRef>> {
        let roots = {
            let mut files = FileTreeWalker::new(files);
            let mut current_dir = self.written_files.root_dir();
            while let Some(file) = files.next() {
                match file {
                    TreeWalkerItem::EnterDirectory(dir) => {
                        let name = dir.name();
                        let dir = self.written_files.get_mut(&current_dir);
                        current_dir.push(dir.push_dir(name));
                    }
                    TreeWalkerItem::ExitDirectory(_dir) => {
                        let dir = self.written_files.get_mut(&current_dir);
                        for &level in &self.entry_types {
                            Self::write_directory(&mut self.data, level, dir)?;
                        }
                        current_dir.pop();
                    }
                    TreeWalkerItem::File(file) => {
                        if let File::File { name, contents } = file {
                            let start = self.data.pad_align_sector()?;
                            self.data.write_all(&contents)?;
                            let dir = self.written_files.get_mut(&current_dir);
                            dir.files.push(WrittenFile {
                                name: name.clone(),
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
            let dir = self.written_files.get_mut(&current_dir);
            for ty in &self.entry_types {
                Self::write_directory(&mut self.data, *ty, dir)?;
            }

            self.written_files.root_refs().clone()
        };

        let pos = self.data.stream_position()?;
        for (_ty, root) in &roots {
            self.update_directory(*root, *root)?;
        }
        // We need to seek back to this position
        self.data.seek(SeekFrom::Start(pos))?;

        Ok(roots)
    }

    fn write_path_tables(&mut self) -> io::Result<()> {
        for i in 0..self.entry_types.len() {
            let ty = self.entry_types[i];
            let l_ref = self.write_path_table(ty, EndianType::LittleEndian)?;
            let m_ref = self.write_path_table(ty, EndianType::BigEndian)?;
            assert_eq!(l_ref.size, m_ref.size);
            self.path_tables.insert(
                ty,
                PathTableRef {
                    lpt: l_ref.extent,
                    mpt: m_ref.extent,
                    size: l_ref.size as u64,
                },
            );
        }
        Ok(())
    }

    fn write_pt_entry(
        &mut self,
        path: &Vec<Arc<String>>,
        parent_ref: &DirectoryRef,
        parent_number: u16,
        endian: EndianType,
    ) -> io::Result<()> {
        // We add 1 for each path component for the leading slash, but we don't start with one, so
        // we remove 1
        let total_len: usize = path.iter().map(|s| s.len() + 1).sum::<usize>() - 1;
        let header = PathTableEntryHeader {
            len: total_len as u8,
            extended_attr_record: 0,
            parent_directory_number: endian.u16_bytes(parent_number),
            parent_lba: endian.u32_bytes(parent_ref.extent.0 as u32),
        };
        self.data.write_all(bytemuck::bytes_of(&header))?;
        let mut is_first = true;
        for part in path {
            if !is_first {
                self.data.write_all(&[b'/'])?;
            } else {
                is_first = false;
            }
            self.data.write_all(part.as_bytes())?;
        }
        if total_len % 2 == 1 {
            self.data.write_all(&[0])?;
        }
        Ok(())
    }

    fn write_path_table(&mut self, ty: EntryType, endian: EndianType) -> io::Result<DirectoryRef> {
        let start = self.data.pad_align_sector()?;
        PathTableWriter {
            written_files: &mut self.written_files,
            ty,
            endian,
        }
        .write(&mut self.data)?;
        let size = self.data.stream_position()? as usize - (start.0 * self.data.sector_size);
        let _end = self.data.pad_align_sector()?;
        Ok(DirectoryRef {
            extent: start,
            size,
        })
    }

    /// Writes the partition tables (MBR, GPT, or Hybrid) based on configuration.
    fn write_partition_tables(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        // Calculate disk size in 512-byte sectors (for MBR/GPT compatibility)
        let disk_size_512 = (end_sector.0 * self.data.sector_size / 512) as u64;

        match self.ops.features.hybrid_boot.as_ref().map(|h| h.partition_scheme) {
            None | Some(PartitionScheme::None) => {
                // No partition table - write a minimal MBR for basic compatibility
                // This is the legacy behavior
                self.write_legacy_mbr(end_sector)?;
            }
            Some(PartitionScheme::Mbr) => {
                self.write_mbr_boot(end_sector)?;
            }
            Some(PartitionScheme::Gpt) => {
                self.write_gpt_boot(end_sector, disk_size_512)?;
            }
            Some(PartitionScheme::Hybrid) => {
                self.write_hybrid_boot(end_sector, disk_size_512)?;
            }
        }

        Ok(())
    }

    /// Writes a legacy MBR with a protective partition (current behavior).
    fn write_legacy_mbr(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let start_sector = LogicalSector(16);
        let start_block = (start_sector.0 * (self.data.sector_size / 512)) as u32;
        let end_block = (end_sector.0 * (self.data.sector_size / 512)) as u32;

        let mut mbr = MasterBootRecord::default();
        mbr.with_partition_table(|pt| {
            pt[0] = MbrPartition {
                boot_indicator: 0x80,
                start_chs: Chs::new(start_block),
                part_type: MbrPartitionType::Iso9660.to_u8(),
                end_chs: Chs::new(end_block),
                start_lba: start_block,
                sector_count: end_block - start_block,
            };
        });

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot {
            if let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap {
                let len = bootstrap.len().min(446);
                mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
            }
        }

        self.data.seek(SeekFrom::Start(0))?;
        self.data.write_all(bytemuck::bytes_of(&mbr))?;

        Ok(())
    }

    /// Writes an MBR partition table for BIOS USB boot (isohybrid-style).
    fn write_mbr_boot(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let end_block = (end_sector.0 * (self.data.sector_size / 512)) as u32;

        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        let mut mbr = MasterBootRecord::default();
        mbr.with_partition_table(|pt| {
            // Create a partition covering the entire ISO
            // Type 0x17 is ISO9660/Hidden NTFS which is commonly used for hybrid ISOs
            pt[0] = MbrPartition {
                boot_indicator: if bootable { 0x80 } else { 0x00 },
                start_chs: Chs::new(0),
                part_type: MbrPartitionType::Iso9660.to_u8(),
                end_chs: Chs::new(end_block.saturating_sub(1)),
                start_lba: 0,
                sector_count: end_block,
            };
        });

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot {
            if let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap {
                let len = bootstrap.len().min(446);
                mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
            }
        }

        self.data.seek(SeekFrom::Start(0))?;
        self.data.write_all(bytemuck::bytes_of(&mbr))?;

        Ok(())
    }

    /// Writes a GPT partition table for UEFI boot.
    fn write_gpt_boot(&mut self, _end_sector: LogicalSector, disk_size_512: u64) -> io::Result<()> {
        // For GPT, we need:
        // 1. Protective MBR at sector 0
        // 2. Primary GPT header at sector 1
        // 3. GPT partition entries at sectors 2-33 (128 entries * 128 bytes = 32 sectors)
        // 4. Backup GPT entries and header at end of disk

        // Write protective MBR
        let mbr = MasterBootRecord::protective(disk_size_512);
        self.data.seek(SeekFrom::Start(0))?;
        self.data.write_all(bytemuck::bytes_of(&mbr))?;

        // Create GPT partition entry for the ISO data
        // Start after GPT structures (sector 34 in 512-byte sectors)
        let iso_start_lba = 34u64;
        let iso_end_lba = disk_size_512.saturating_sub(34); // Leave room for backup GPT

        // Create a deterministic partition GUID based on the volume name
        let partition_guid = Self::generate_guid_from_string(&self.ops.volume_name);
        let disk_guid = Self::generate_guid_from_string(&alloc::format!("disk-{}", self.ops.volume_name));

        let mut entries = [GptPartitionEntry::default(); 4];
        entries[0] = GptPartitionEntry::new(
            Guid::BASIC_DATA, // or could use a custom ISO GUID
            partition_guid,
            iso_start_lba,
            iso_end_lba,
        );

        // Calculate CRC32 of partition entries
        let entries_bytes = bytemuck::bytes_of(&entries);
        let entries_crc = Self::crc32(entries_bytes);

        // Create and write primary GPT header
        let header_bytes = Self::write_gpt_header_bytes(
            disk_guid,
            1,                  // my_lba (primary is at sector 1)
            disk_size_512 - 1,  // alternate_lba (backup at last sector)
            iso_start_lba,      // first_usable_lba
            iso_end_lba,        // last_usable_lba
            2,                  // partition_entry_lba
            4,                  // num_partition_entries
            entries_crc,
        );

        // Write primary GPT header
        self.data.seek(SeekFrom::Start(512))?; // Sector 1
        self.data.write_all(&header_bytes)?;

        // Write partition entries (starting at sector 2)
        self.data.seek(SeekFrom::Start(1024))?; // Sector 2
        self.data.write_all(entries_bytes)?;

        // Note: In a full implementation, we'd also write the backup GPT at the end
        // For now, we skip this as ISOs are typically read-only

        Ok(())
    }

    /// Writes a Hybrid MBR + GPT for dual BIOS/UEFI boot.
    fn write_hybrid_boot(&mut self, _end_sector: LogicalSector, disk_size_512: u64) -> io::Result<()> {
        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        // Create GPT partition entry for the ISO
        let iso_start_lba = 34u64;
        let iso_end_lba = disk_size_512.saturating_sub(34);

        // Create deterministic GUIDs
        let partition_guid = Self::generate_guid_from_string(&self.ops.volume_name);
        let disk_guid = Self::generate_guid_from_string(&alloc::format!("disk-{}", self.ops.volume_name));

        let gpt_entries = [
            GptPartitionEntry::new(
                Guid::BASIC_DATA,
                partition_guid,
                iso_start_lba,
                iso_end_lba,
            ),
            GptPartitionEntry::default(),
        ];

        // Build hybrid MBR using hadris-part
        let mut mbr = HybridMbrBuilder::new(disk_size_512)
            .protective_slot(0)
            .mirror_partition(0, MbrPartitionType::Iso9660, bootable)
            .build(&gpt_entries)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, alloc::format!("{:?}", e)))?;

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot {
            if let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap {
                let len = bootstrap.len().min(446);
                mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
            }
        }

        // Write hybrid MBR
        self.data.seek(SeekFrom::Start(0))?;
        self.data.write_all(bytemuck::bytes_of(&mbr))?;

        // Calculate CRC32 of partition entries
        let entries_bytes = bytemuck::bytes_of(&gpt_entries);
        let entries_crc = Self::crc32(entries_bytes);

        // Create and write primary GPT header
        let header_bytes = Self::write_gpt_header_bytes(
            disk_guid,
            1,
            disk_size_512 - 1,
            iso_start_lba,
            iso_end_lba,
            2,
            2, // Only 2 entries in our case
            entries_crc,
        );

        // Write primary GPT header
        self.data.seek(SeekFrom::Start(512))?;
        self.data.write_all(&header_bytes)?;

        // Write partition entries
        self.data.seek(SeekFrom::Start(1024))?;
        self.data.write_all(entries_bytes)?;

        Ok(())
    }

    /// Simple CRC32 calculation for GPT.
    fn crc32(data: &[u8]) -> u32 {
        // Using the standard CRC-32 polynomial
        let mut crc = !0u32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB88320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// Generates a deterministic GUID from a string (simple hash-based).
    fn generate_guid_from_string(s: &str) -> Guid {
        // Simple FNV-1a hash to generate a deterministic GUID
        let mut hash1: u64 = 0xcbf29ce484222325;
        let mut hash2: u64 = 0x100000001b3;

        for byte in s.bytes() {
            hash1 ^= byte as u64;
            hash1 = hash1.wrapping_mul(0x100000001b3);
            hash2 ^= byte as u64;
            hash2 = hash2.wrapping_mul(0xcbf29ce484222325);
        }

        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&hash1.to_le_bytes());
        bytes[8..16].copy_from_slice(&hash2.to_le_bytes());

        // Set version 4 (random) and variant bits
        bytes[6] = (bytes[6] & 0x0f) | 0x40; // Version 4
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // Variant 1

        Guid::from_bytes(bytes)
    }

    /// Writes a GPT header to the given buffer (92 bytes).
    fn write_gpt_header_bytes(
        disk_guid: Guid,
        my_lba: u64,
        alternate_lba: u64,
        first_usable_lba: u64,
        last_usable_lba: u64,
        partition_entry_lba: u64,
        num_partition_entries: u32,
        partition_entry_array_crc32: u32,
    ) -> [u8; 92] {
        let mut buf = [0u8; 92];

        // Signature: "EFI PART"
        buf[0..8].copy_from_slice(b"EFI PART");
        // Revision: 1.0
        buf[8..12].copy_from_slice(&0x00010000u32.to_le_bytes());
        // Header size: 92
        buf[12..16].copy_from_slice(&92u32.to_le_bytes());
        // Header CRC32: placeholder, will be calculated
        buf[16..20].copy_from_slice(&0u32.to_le_bytes());
        // Reserved
        buf[20..24].copy_from_slice(&0u32.to_le_bytes());
        // My LBA
        buf[24..32].copy_from_slice(&my_lba.to_le_bytes());
        // Alternate LBA
        buf[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
        // First usable LBA
        buf[40..48].copy_from_slice(&first_usable_lba.to_le_bytes());
        // Last usable LBA
        buf[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
        // Disk GUID
        buf[56..72].copy_from_slice(&disk_guid.to_bytes());
        // Partition entry LBA
        buf[72..80].copy_from_slice(&partition_entry_lba.to_le_bytes());
        // Number of partition entries
        buf[80..84].copy_from_slice(&num_partition_entries.to_le_bytes());
        // Size of partition entry: 128
        buf[84..88].copy_from_slice(&128u32.to_le_bytes());
        // Partition entry array CRC32
        buf[88..92].copy_from_slice(&partition_entry_array_crc32.to_le_bytes());

        // Calculate and set header CRC32
        let crc = Self::crc32(&buf);
        buf[16..20].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    fn update_directory(
        &mut self,
        parent: DirectoryRef,
        directory: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent)?;
        let mut offset = 0;
        loop {
            if offset >= directory.size as u64 {
                break;
            }
            self.data.seek(SeekFrom::Start(start + offset))?;
            let mut record = DirectoryRecord::parse(&mut self.data)?;
            if record.header().len == 0 {
                break;
            }

            if record.name() == b"\x00" || record.name() == b"\x01" {
                let dir_ref = [directory, parent][record.name()[0] as usize];
                let header = record.header_mut();
                header.extent.write(dir_ref.extent.0 as u32);
                header.data_len.write(dir_ref.size as u32);
                self.data.seek(SeekFrom::Start(start + offset))?;
                record.write(&mut self.data)?;
                offset += record.header().len as u64;
                continue;
            }
            offset += record.header().len as u64;

            if FileFlags::from_bits_truncate(record.header().flags).contains(FileFlags::DIRECTORY) {
                let record = DirectoryRef {
                    extent: LogicalSector(record.header().extent.read() as usize),
                    size: record.header().data_len.read() as usize,
                };
                self.update_directory(directory, record)?;
            }
        }

        Ok(())
    }

    fn write_directory(
        data: &mut IsoCursor<DATA>,
        ty: EntryType,
        dir: &mut WrittenDirectory,
    ) -> io::Result<()> {
        let start = data.pad_align_sector()?;
        // Current Directory Entry
        DirectoryRecord::new(b"\x00", &[], DirectoryRef::default(), FileFlags::DIRECTORY)
            .write(&mut *data)?;

        // Parent Directory Entry
        DirectoryRecord::new(b"\x01", &[], DirectoryRef::default(), FileFlags::DIRECTORY)
            .write(&mut *data)?;

        for directory in &dir.dirs {
            let WrittenDirectory { name, entries, .. } = directory;
            let flags = FileFlags::DIRECTORY;
            let converted_name = ty.convert_name(name);
            let record = DirectoryRecord::new(
                converted_name.as_bytes(),
                &[],
                *entries.get(&ty).unwrap(),
                flags,
            );
            record.write(&mut *data)?;
        }

        for file in &dir.files {
            let WrittenFile { name, entry } = file;
            let flags = FileFlags::empty();
            let converted_name = ty.convert_name(name);
            let su: &[u8] = &[];
            let record = DirectoryRecord::new(converted_name.as_bytes(), su, *entry, flags);
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
            name: Arc::new(String::from("root/dir1/fileA.txt")),
            contents: Vec::new(),
        };
        let file_b = File::File {
            name: Arc::new(String::from("root/dir1/fileB.txt")),
            contents: Vec::new(),
        };
        let file_c = File::File {
            name: Arc::new(String::from("root/fileC.txt")),
            contents: Vec::new(),
        };
        let file_d = File::File {
            name: Arc::new(String::from("root/dir2/fileD.txt")),
            contents: Vec::new(),
        };
        let file_e = File::File {
            name: Arc::new(String::from("root/dir2/subdir/fileE.txt")),
            contents: Vec::new(),
        };

        let subdir_node = File::Directory {
            name: Arc::new(String::from("root/dir2/subdir")),
            children: vec![file_e.clone()],
        };

        let dir1_node = File::Directory {
            name: Arc::new(String::from("root/dir1")),
            children: vec![file_a.clone(), file_b.clone()],
        };

        let dir2_node = File::Directory {
            name: Arc::new(String::from("root/dir2")),
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

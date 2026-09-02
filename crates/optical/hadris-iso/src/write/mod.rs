use alloc::{collections::BTreeMap, sync::Arc};

mod types;
mod utils;

pub mod estimator;
/// APIs for writer.
pub mod writer;

use super::super::boot::{
    BootCatalog, BootInfoTable, BootSectionEntry, ElToritoWriter, Grub2BootInfoTable, PlatformId,
};
use super::super::directory::{DirectoryRecord, DirectoryRef, FileFlags};
use super::super::io::{self, Read, Seek, SeekFrom, Write};
use super::super::io::{IsoCursor, LogicalSector};
use super::super::path::PathTableRef;
use super::super::rrip::RripOptions;
use super::super::volume::{
    BootRecordVolumeDescriptor, PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor,
    VolumeDescriptor, VolumeDescriptorHeader, VolumeDescriptorList, VolumeDescriptorType,
};
use crate::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use crate::directory::RootDirectoryEntry;
use crate::file::EntryType;
use crate::joliet::JolietLevel;
use crate::types::{Charset, IsoStr};
use crate::write::utils::*;
use crate::write::writer::DirectoryId;
use hadris_common::types::endian::{Endian, EndianType};
use hadris_part::{
    GptDisk, GptDiskWriteExt,
    gpt::{GptPartitionEntry, Guid},
    hybrid::HybridMbrBuilder,
    mbr::{MasterBootRecord, MbrPartition, MbrPartitionType},
};
use options::PartitionScheme;
use writer::{DirectoryRelocation, PathTableWriter, WrittenDirectory, WrittenFiles};

use alloc::{string::String, vec, vec::Vec};

use types::*;
pub use types::{File, InputEntry, InputEntryKind, InputFiles, InputMetadata, InputTree};

/// APIs for options.
pub mod options;
use options::IsoFormatOptions;

#[derive(Debug, thiserror::Error)]
/// Identifies a FileConversionError value.
pub enum FileConversionError {
    #[error("I/O error: {0}")]
    /// The `Io` variant.
    Io(#[from] std::io::Error),
    #[error("Path {0:?} is not a valid UTF-8 string")]
    /// The `InvalidUtf8Path` variant.
    InvalidUtf8Path(std::path::PathBuf),
    #[error("Unsupported filesystem entry type at {0:?}")]
    /// The `UnsupportedFileType` variant.
    UnsupportedFileType(std::path::PathBuf),
}

#[derive(Debug, thiserror::Error)]
/// Identifies a IsoCreationError value.
pub enum IsoCreationError {
    #[error(transparent)]
    /// The `Io` variant.
    Io(#[from] io::Error),
}

/// Canonical error for ISO creation operations.
pub type Error = IsoCreationError;
/// Canonical result for ISO creation operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Writer for creating ISO 9660 images.
///
/// Supports:
/// - ISO Level 1, 2, and 3
/// - Joliet extensions (Unicode filenames)
/// - Rock Ridge extensions (POSIX metadata, symlinks, devices)
/// - El Torito bootable images
/// - MBR, GPT, and Hybrid partition tables
/// - Multi-extent files (>4 GiB)
pub struct IsoImageWriter<DATA: Read + Write + Seek> {
    data: IsoCursor<DATA>,
    entry_types: Vec<EntryType>,
    ops: IsoFormatOptions,
    written_files: WrittenFiles,
    path_tables: BTreeMap<EntryType, PathTableRef>,
    inode_counter: u32,
    rrip_time: RripTime,
}

io_transform! {
impl<DATA: Read + Write + Seek> IsoImageWriter<DATA> {
    const VOLUME_DESCRIPTOR_SET_START: LogicalSector = LogicalSector(16);
    const GPT_PARTITION_NAME_ISO: &[u8] = b"ISO9660";
    const GPT_PARTITION_NAME_ESP: &[u8] = b"EFI System Partition";
    const BOOT_CATALOG_FILENAME: &str = "boot.catalog";

    /// Creates a complete ISO image and returns its output target.
    pub async fn create<T: Into<InputTree>>(
        data: DATA,
        files: T,
        ops: IsoFormatOptions,
    ) -> Result<DATA> {
        Self::create_with_allocation_floor(data, files, ops, None).await
    }

    /// Creates an ISO image while keeping file and directory allocations at or
    /// after an optional logical-sector floor.
    ///
    /// This is useful when composing ISO 9660 with another on-disc format whose
    /// metadata occupies sectors after the ISO volume descriptor sequence. A
    /// floor below the end of the descriptors has no effect. Volume descriptors
    /// and system-area structures retain their specification-defined locations.
    pub async fn create_with_allocation_floor<T: Into<InputTree>>(
        data: DATA,
        files: T,
        ops: IsoFormatOptions,
        allocation_floor: Option<u32>,
    ) -> Result<DATA> {
        let mut files = files.into();
        if ops.sector_size != 2048 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ISO creation currently requires 2048-byte logical sectors",
            )
            .into());
        }

        files.validate(ops.features.rock_ridge.as_ref())?;

        let mut writer = Self::new(data, ops);

        writer.write_volume_descriptors(&mut files).await?;
        writer.try_apply_allocation_floor(allocation_floor)?;

        let root_dirs = writer.write_files(&files).await?;
        writer.write_path_tables().await?;
        writer.finalize_volume_descriptors(root_dirs).await?;

        Ok(writer.into_inner())
    }

    async fn try_apply_allocation_floor(&mut self, floor: Option<u32>) -> io::Result<()> {
        let Some(sector) = floor else { return Ok(()) };

        let current = self
            .data
            .stream_position()
            .await
            .map_err(io::Error::erase)?;

        let target = u64::from(sector)
            .checked_mul(self.ops.sector_size as u64)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "allocation floor overflow")
            })?;

        if target > current {
            self.data
                .seek(SeekFrom::Start(target))
                .await
                .map_err(io::Error::erase)?;
        }

        Ok(())
    }

    /// Returns the output target.
    pub fn into_inner(self) -> DATA {
        self.data.into_inner()
    }

    fn new(data: DATA, ops: IsoFormatOptions) -> Self {
        let now = super::super::directory::DirDateTime::now();
        let rrip_time = *<&RripTime>::try_from(bytemuck::bytes_of(&now)).unwrap();
        let entry_types = ops.entry_types();

        Self {
            data: IsoCursor::new(data, ops.sector_size),
            ops,
            entry_types,
            written_files: WrittenFiles::new(),
            path_tables: BTreeMap::new(),
            inode_counter: 1,
            rrip_time,
        }
    }

    fn parse_iso_str<C: Charset, const N: usize>(
        &self,
        s: &str,
        field_name: &'static str,
    ) -> io::Result<IsoStr<C, N>> {
        if self.ops.strict_charset {
            IsoStr::from_str_lossy(s)
        } else {
            IsoStr::from_str_unchecked(s)
        }
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, field_name))
    }

    async fn write_volume_descriptors(&mut self, files: &mut InputTree) -> io::Result<()> {
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START).await?;
        let mut volume_descriptors = VolumeDescriptorList::empty();

        for &entry in &self.entry_types {
            let descriptor = self.create_volume_descriptor(entry)?;
            volume_descriptors.push(descriptor);
        }

        if let Some(boot) = &self.ops.features.el_torito {
            let boot_record = ElToritoWriter::create_descriptor(boot, files);
            volume_descriptors.insert(1, VolumeDescriptor::BootRecord(boot_record));
        }

        volume_descriptors.write(&mut self.data).await?;

        Ok(())
    }

     fn create_volume_descriptor(&self, entry: EntryType) -> io::Result<VolumeDescriptor> {
        match entry {
            EntryType::Level1 { .. } | EntryType::Level2 { .. } => {
                Ok(VolumeDescriptor::Primary(self.create_primary_volume_descriptor()?))
            }
            EntryType::Level3 { .. } => {
                Ok(VolumeDescriptor::Supplementary(self.create_enhanced_volume_descriptor()?))
            }
            EntryType::Joliet { level, .. } => {
                Ok(VolumeDescriptor::Supplementary(self.create_joliet_volume_descriptor(level)?))
            }
        }
    }

    fn configure_base_directory_record(&self, record: &mut RootDirectoryEntry) {
        record.header.len = 34;
        record.header.flags = FileFlags::DIRECTORY.bits();
        record.header.file_identifier_len = 1;
        record.header.volume_sequence_number.write(1);
    }

    fn create_primary_volume_descriptor(&self) -> io::Result<PrimaryVolumeDescriptor> {
        let mut pvd = PrimaryVolumeDescriptor::new(&self.ops.volume_name, 0);
        pvd.volume_identifier = self.parse_iso_str(&self.ops.volume_name, "volume name")?;
        self.configure_base_directory_record(&mut pvd.dir_record);
        pvd.volume_sequence_number.write(1);

        if let Some(s) = &self.ops.system_id {
            pvd.system_identifier = self.parse_iso_str(s, "system identifier")?;
        }
        if let Some(s) = &self.ops.volume_set_id {
            pvd.volume_set_identifier = self.parse_iso_str(s, "volume set identifier")?;
        }
        if let Some(s) = &self.ops.publisher_id {
            pvd.publisher_identifier = self.parse_iso_str(s, "publisher identifier")?;
        }
        if let Some(s) = &self.ops.preparer_id {
            pvd.preparer_identifier = self.parse_iso_str(s, "preparer identifier")?;
        }
        if let Some(s) = &self.ops.application_id {
            pvd.application_identifier = self.parse_iso_str(s, "application identifier")?;
        }

        Ok(pvd)
    }

    fn create_enhanced_volume_descriptor(&self) -> io::Result<SupplementaryVolumeDescriptor> {
        let mut evd = SupplementaryVolumeDescriptor::new_evd(&self.ops.volume_name, 0);
        evd.volume_identifier = self.parse_iso_str(&self.ops.volume_name, "volume name")?;
        self.configure_base_directory_record(&mut evd.dir_record);
        evd.volume_sequence_number.write(1);
        Ok(evd)
    }

    fn create_joliet_volume_descriptor(&self, level: JolietLevel) -> io::Result<SupplementaryVolumeDescriptor> {
        let mut svd = SupplementaryVolumeDescriptor::new_svd(
            &self.ops.volume_name,
            0,
            level.escape_sequence(),
        );
        self.configure_base_directory_record(&mut svd.dir_record);
        svd.volume_sequence_number.write(1);

        if let Some(s) = &self.ops.system_id {
            svd.system_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
        }
        if let Some(s) = &self.ops.volume_set_id {
            svd.volume_set_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
        }
        if let Some(s) = &self.ops.publisher_id {
            svd.publisher_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
        }
        if let Some(s) = &self.ops.preparer_id {
            svd.preparer_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
        }
        if let Some(s) = &self.ops.application_id {
            svd.application_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
        }

        Ok(svd)
    }

    async fn finalize_volume_descriptors(
        &mut self,
        root_dirs: BTreeMap<EntryType, DirectoryRef>,
    ) -> io::Result<()> {
        let catalog_ptr = self.write_boot_catalog().await?;
        let end_sector = self.pad_and_get_end_sector().await?;
        let volume_space = self.volume_space_sectors(end_sector);

        self.patch_volume_descriptors(&root_dirs, catalog_ptr, volume_space).await?;

        self.write_partition_tables(end_sector).await?;
        Ok(())
    }

    async fn write_boot_section(
        &mut self,
        catalog: &mut BootCatalog,
        section: Option<&BootSectionOptions>,
        entry: &BootEntryOptions,
    ) -> io::Result<()> {
        let dir_ref = self.find_boot_image(&entry.boot_image_path)?;
        let load_size = self.calculate_load_size(entry, &dir_ref);
        let boot_entry = BootSectionEntry::new(entry.emulation, 0, load_size, dir_ref.extent.0 as u32);

        if let Some(section) = section {
            catalog.add_section(section.platform, vec![boot_entry]);
        } else {
            catalog.set_default_entry(boot_entry);
        }

        if entry.boot_info_table || entry.grub2_boot_info {
            self.write_boot_info_table(entry, &dir_ref).await?;
        }

        Ok(())
    }

    fn find_boot_image(&self, path: &str) -> io::Result<DirectoryRef> {
        self.written_files
            .find_file(path, self.ops.path_separator)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "boot image file not found"))
    }

    fn calculate_load_size(&self, entry: &BootEntryOptions, dir_ref: &DirectoryRef) -> u16 {
        entry.load_size.map(|s| s.get()).unwrap_or_else(|| {
            if entry.emulation.is_emulated() {
                1
            } else {
                dir_ref.size.div_ceil(512) as u16
            }
        })
    }

    async fn write_boot_catalog(&mut self) -> io::Result<Option<u32>> {
        let Some(boot) = self.ops.features.el_torito.clone() else { return Ok(None) };

        let mut catalog = BootCatalog::default();
        let current_sector = self.data.pad_align_sector().await?;

        for (section, entry) in boot.sections() {
            self.write_boot_section(&mut catalog, section.as_ref(), &entry).await?;
        }

        self.write_catalog_to_disk(&boot, &mut catalog, current_sector).await
    }

    async fn write_boot_info_table(&mut self, entry: &BootEntryOptions, dir_ref: &DirectoryRef) -> io::Result<()> {
        if dir_ref.size < 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "boot image too small for boot info table (minimum 64 bytes)",
            ));
        }

        let checksum = self.calculate_boot_checksum(dir_ref).await?;
        let byte_offset = (dir_ref.extent.0 as u64) * self.ops.sector_size as u64;

        self.data
            .seek(SeekFrom::Start(byte_offset + 8))
            .await
            .map_err(io::Error::erase)?;

        if entry.grub2_boot_info {
            let table = Grub2BootInfoTable::new(
                dir_ref.extent.0 as u32,
                dir_ref.size as u32,
                checksum,
            );
            self.data.write_all(bytemuck::bytes_of(&table)).await?;
        } else {
            let table = BootInfoTable::new(
                dir_ref.extent.0 as u32,
                dir_ref.size as u32,
                checksum,
            );
            self.data.write_all(bytemuck::bytes_of(&table)).await?;
        }

        Ok(())
    }

    async fn calculate_boot_checksum(&mut self, dir_ref: &DirectoryRef) -> io::Result<u32> {
        let mut checksum = 0u32;
        let mut buffer = [0u8; 4];
        let byte_offset = (dir_ref.extent.0 as u64) * self.ops.sector_size as u64;

        self.data
            .seek(SeekFrom::Start(byte_offset + 64))
            .await
            .map_err(io::Error::erase)?;

        let checksum_bytes = dir_ref.size - 64;
        for _ in 0..(checksum_bytes / 4) {
            self.data.read_exact(&mut buffer).await?;
            checksum = checksum.wrapping_add(u32::from_le_bytes(buffer));
        }

        Ok(checksum)
    }

    async fn write_catalog_to_disk(
        &mut self,
        boot: &BootOptions,
        catalog: &mut BootCatalog,
        current_sector: LogicalSector,
    ) -> io::Result<Option<u32>> {
        if boot.write_boot_catalog {
            let dir_ref = self.written_files
                .find_file(Self::BOOT_CATALOG_FILENAME, self.ops.path_separator)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "boot.catalog not found"))?;

            self.data.seek_sector(dir_ref.extent).await?;
            if dir_ref.size < catalog.size() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "boot.catalog too small"));
            }
            catalog.write(&mut self.data).await?;
            self.data.seek_sector(current_sector).await?;
            Ok(Some(dir_ref.extent.0 as u32))
        } else {
            self.data.seek_sector(current_sector).await?;
            catalog.write(&mut self.data).await?;
            self.data.pad_align_sector().await?;
            Ok(Some(current_sector.0 as u32))
        }
    }

    async fn pad_and_get_end_sector(&mut self) -> io::Result<LogicalSector> {
        let end_position = self.data.stream_position().await.map_err(io::Error::erase)?;
        let end_sector = self.data.pad_align_sector().await?;
        let image_len = end_sector.0 as u64 * self.ops.sector_size as u64;

        if alignment_requires_materialization(end_position, image_len) {
            self.data
                .seek(SeekFrom::Start(image_len - 1))
                .await
                .map_err(io::Error::erase)?;
            self.data.write_all(&[0]).await?;
        }

        Ok(end_sector)
    }

    async fn patch_volume_descriptors(
        &mut self,
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        catalog_ptr: Option<u32>,
        volume_space: u32,
    ) -> io::Result<()> {
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START).await?;

        let mut buffer = vec![0u8; self.ops.sector_size];
        loop {
            self.data.read_exact(&mut buffer).await?;
            let header = VolumeDescriptorHeader::from_bytes(&buffer[0..7]);
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);

            if matches!(ty, VolumeDescriptorType::VolumeSetTerminator) {
                break;
            }

            if !header.is_valid() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid volume descriptor header during finalization",
                ));
            }

            self.patch_single_descriptor(&mut buffer, ty, root_dirs, catalog_ptr, volume_space)?;

            self.data
                .seek_relative(-(buffer.len() as i64))
                .await
                .map_err(io::Error::erase)?;
            self.data.write_all(&buffer).await?;
        }

        Ok(())
    }

    fn patch_single_descriptor(
        &self,
        buffer: &mut [u8],
        ty: VolumeDescriptorType,
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        catalog_ptr: Option<u32>,
        volume_space: u32,
    ) -> io::Result<()> {
        match ty {
            VolumeDescriptorType::PrimaryVolumeDescriptor => {
                self.patch_primary_descriptor(buffer, root_dirs, volume_space)
            }
            VolumeDescriptorType::SupplementaryVolumeDescriptor => {
                self.patch_supplementary_descriptor(buffer, root_dirs, volume_space)
            }
            VolumeDescriptorType::BootRecord => {
                self.patch_boot_record(buffer, catalog_ptr)
            }
            _ => Ok(()),
        }
    }

    fn patch_primary_descriptor(
        &self,
        buffer: &mut [u8],
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        volume_space: u32,
    ) -> io::Result<()> {
        let base_type = self
            .entry_types
            .iter()
            .find(|e| matches!(e, EntryType::Level1 { .. } | EntryType::Level2 { .. }))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "no base Level entry type found for PVD")
            })?;

        let root_dir = root_dirs.get(base_type)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "root directory not found for PVD"))?;
        let pt = self.path_tables.get(base_type)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path table not found for PVD"))?;

        let pvd = bytemuck::from_bytes_mut::<PrimaryVolumeDescriptor>(buffer);
        pvd.dir_record.header.extent.write(root_dir.extent.0 as u32);
        pvd.dir_record.header.data_len.write(root_dir.size as u32);
        pvd.type_l_path_table.set(pt.lpt.0 as u32);
        pvd.type_m_path_table.set(pt.mpt.0 as u32);
        pvd.path_table_size.write(pt.size as u32);
        pvd.volume_space_size.write(volume_space);

        Ok(())
    }

    fn patch_supplementary_descriptor(
        &self,
        buffer: &mut [u8],
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        volume_space: u32,
    ) -> io::Result<()> {
        let svd = bytemuck::from_bytes_mut::<SupplementaryVolumeDescriptor>(buffer);

        match svd.header.version {
            1 => self.patch_joliet_descriptor(svd, root_dirs, volume_space),
            2 => self.patch_enhanced_descriptor(svd, root_dirs, volume_space),
            _ => Ok(()),
        }
    }

    fn patch_joliet_descriptor(
        &self,
        svd: &mut SupplementaryVolumeDescriptor,
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        volume_space: u32,
    ) -> io::Result<()> {
        for &level in JolietLevel::all() {
            if svd.escape_sequences != level.escape_sequence() {
                continue;
            }

            let joliet = self
                .entry_types
                .iter()
                .find(|e| matches!(e, EntryType::Joliet{ level: jl, ..} if *jl == level))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Joliet entry type not found"))?;

            let root_dir = root_dirs.get(joliet)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "root dir not found for Joliet"))?;
            let pt = self.path_tables.get(joliet)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path table not found for Joliet"))?;

            svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
            svd.dir_record.header.data_len.write(root_dir.size as u32);
            svd.type_l_path_table.set(pt.lpt.0 as u32);
            svd.type_m_path_table.set(pt.mpt.0 as u32);
            svd.path_table_size.write(pt.size as u32);
            svd.volume_space_size.write(volume_space);
            break;
        }

        Ok(())
    }

    fn patch_enhanced_descriptor(
        &self,
        svd: &mut SupplementaryVolumeDescriptor,
        root_dirs: &BTreeMap<EntryType, DirectoryRef>,
        volume_space: u32,
    ) -> io::Result<()> {
        if svd.escape_sequences != [b' '; 32] {
            return Ok(());
        }

        let l3 = self
            .entry_types
            .iter()
            .find(|e| matches!(e, EntryType::Level3 { .. }))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Level3 entry type not found"))?;

        let root_dir = root_dirs.get(l3)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "root dir not found for Level3"))?;
        let pt = self.path_tables.get(l3)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path table not found for Level3"))?;

        svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
        svd.dir_record.header.data_len.write(root_dir.size as u32);
        svd.type_l_path_table.set(pt.lpt.0 as u32);
        svd.type_m_path_table.set(pt.mpt.0 as u32);
        svd.path_table_size.write(pt.size as u32);
        svd.volume_space_size.write(volume_space);

        Ok(())
    }

    fn patch_boot_record(&self, buffer: &mut [u8], catalog_ptr: Option<u32>) -> io::Result<()> {
        let Some(ptr) = catalog_ptr else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "boot record found but no boot catalog was written",
            ));
        };

        let boot_record = bytemuck::from_bytes_mut::<BootRecordVolumeDescriptor>(buffer);
        boot_record.catalog_ptr.set(ptr);
        Ok(())
    }

    async fn write_files(&mut self, files: &InputTree) -> io::Result<BTreeMap<EntryType, DirectoryRef>> {
        let walker = FileTreeWalker::new(files);
        walker.walk(&mut self.written_files);

        if self.ops.has_rock_ridge_deep_dirs() {
            relocate_deep_directories(&mut self.written_files);
        }

        let root_id = self.written_files.root_dir();
        let (order, file_order) = collect_preorder(&self.written_files, &root_id);

        let rrip_options = self.ops.features.rock_ridge;
        let rrip_time = self.rrip_time;
        let entry_types = self.entry_types.clone();

        let relocation_refs = self.plan_layout(
            &order, &file_order, &entry_types,
            rrip_options, &rrip_time, &root_id,
        ).await?;

        self.write_layout(
            &order, &file_order, &entry_types,
            rrip_options, &rrip_time, &relocation_refs, &root_id,
        ).await?;

        self.patch_moved_directories(&root_id, &relocation_refs).await?;

        let roots = self.written_files.root_refs().clone();
        let pos = self.data.stream_position().await.map_err(io::Error::erase)?;
        for root in roots.values().cloned() {
            self.update_directory(root, root).await?;
        }
        self.data.seek(SeekFrom::Start(pos)).await.map_err(io::Error::erase)?;

        Ok(roots)
    }

    async fn plan_layout(
        &mut self,
        order: &[DirectoryId],
        file_order: &[FileOrder],
        entry_types: &[EntryType],
        rrip_options: Option<RripOptions>,
        rrip_time: &RripTime,
        root_id: &DirectoryId,
    ) -> io::Result<RelocationMap> {
        let sector_size = self.ops.sector_size as u64;
        let cursor = self.data.stream_position().await.map_err(io::Error::erase)?;
        let inode_counter = self.inode_counter;

        let default_refs = self.build_default_refs(order, entry_types);
        let (relocation_refs, cursor, inode_counter) = self.plan_directories(
            order,
            entry_types,
            &default_refs,
            rrip_options,
            rrip_time,
            root_id,
            cursor,
            inode_counter,
        ).await?;

        let _ = self.plan_files(file_order, cursor, sector_size).await?;

        self.inode_counter = inode_counter;
        Ok(relocation_refs)
    }

    fn build_default_refs(
        &mut self,
        order: &[DirectoryId],
        entry_types: &[EntryType],
    ) -> RelocationMap {
        let mut default_refs = BTreeMap::new();
        for directory_id in order {
            let dir = self.written_files.get(directory_id);
            for ty in entry_types {
                default_refs.insert((dir.id, *ty), DirectoryRef::default());
                if let DirectoryRelocation::Moved { id, .. } = dir.relocation {
                    default_refs.insert((id, *ty), DirectoryRef::default());
                }
            }
            let dir = self.written_files.get_mut(directory_id);
            for ty in entry_types {
                dir.entries.entry(*ty).or_default();
            }
        }
        default_refs
    }

    #[allow(clippy::too_many_arguments)]
    async fn plan_directories(
        &mut self,
        order: &[DirectoryId],
        entry_types: &[EntryType],
        default_refs: &RelocationMap,
        rrip_options: Option<RripOptions>,
        rrip_time: &RripTime,
        root_id: &DirectoryId,
        mut cursor: u64,
        mut inode_counter: u32,
    ) -> io::Result<(RelocationMap, u64, u32)> {
        let sector_size = self.ops.sector_size as u64;
        let mut relocation_refs = BTreeMap::new();

        for directory_id in order {
            let is_root = directory_id == root_id;
            for ty in entry_types {
                let dir = self.written_files.get(directory_id);
                let records = PendingRecords::new(
                    *ty,
                    dir,
                    is_root,
                    &mut inode_counter,
                    rrip_options.as_ref(),
                    rrip_time,
                    default_refs,
                )?;

                let (extent, size_sectors) = layout_directory_records(cursor, sector_size, &records);
                let ca_len = records.overflow_len();
                let reference = DirectoryRef::new(extent as _, (size_sectors * sector_size) as usize);

                let dir = self.written_files.get_mut(directory_id);
                dir.entries.insert(*ty, reference);
                relocation_refs.insert((dir.id, *ty), reference);

                if let DirectoryRelocation::Moved { id, .. } = dir.relocation {
                    relocation_refs.insert((id, *ty), reference);
                }

                cursor = (extent + size_sectors) * sector_size + ca_len;
            }
        }

        Ok((relocation_refs, cursor, inode_counter))
    }

    async fn plan_files(
        &mut self,
        file_order: &[FileOrder],
        mut cursor: u64,
        sector_size: u64,
    ) -> io::Result<u64> {

        for (directory_id, index) in file_order {
            let dir = self.written_files.get_mut(directory_id);
            let file = &mut dir.files[*index];

            let len = file.kind.file_len().unwrap_or(0);

            if len == 0 {
                file.entry = DirectoryRef::default();
                file.additional_extents.clear();
                continue;
            }

            let extents_iter = ExtentIter::new(len, cursor, sector_size);
            let (mut extents_vec, new_cursor) = extents_iter.collect_with_cursor();

            if !extents_vec.is_empty() {
                let first = extents_vec.remove(0);
                file.entry = first;
                file.additional_extents = extents_vec;
            }

            cursor = new_cursor;
        }

        Ok(cursor)
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_layout(
        &mut self,
        order: &[DirectoryId],
        file_order: &[FileOrder],
        entry_types: &[EntryType],
        rrip_options: Option<RripOptions>,
        rrip_time: &[u8; 7],
        relocation_refs: &RelocationMap,
        root_id: &DirectoryId,
    ) -> io::Result<()> {
        let sector_size = self.ops.sector_size as u64;
        let mut inode_counter = self.inode_counter;

        self.write_directories(
            order,
            entry_types,
            rrip_options,
            rrip_time,
            relocation_refs,
            root_id,
            sector_size,
            &mut inode_counter,
        ).await?;

        self.inode_counter = inode_counter;
        self.write_file_data(file_order).await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_directories(
        &mut self,
        order: &[DirectoryId],
        entry_types: &[EntryType],
        rrip_options: Option<RripOptions>,
        rrip_time: &RripTime,
        relocation_refs: &RelocationMap,
        root_id: &DirectoryId,
        sector_size: u64,
        inode_counter: &mut u32,
    ) -> io::Result<()> {
        for directory_id in order {
            let is_root = directory_id == root_id;
            for ty in entry_types {
                let dir = self.written_files.get(directory_id);
                let expected = dir.entries.get(ty).copied().unwrap_or_default();
                let mut records = PendingRecords::new(
                    *ty,
                    dir,
                    is_root,
                    inode_counter,
                    rrip_options.as_ref(),
                    rrip_time,
                    relocation_refs,
                )?;
                Self::write_directory_records(&mut self.data, sector_size, expected, &mut records).await?;
            }
        }
        Ok(())
    }

    async fn write_file_data(
        &mut self,
        file_order: &[FileOrder],
    ) -> io::Result<()> {

        for (directory_id, index) in file_order {
            let dir = self.written_files.get(directory_id);
            let file = &dir.files[*index];

            let mut offset = 0_u64;
            for extent in file.extents() {
                let start = self.data.pad_align_sector().await?;

                if start != extent.extent {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "extent prediction did not match",
                    ));
                }

                #[cfg(test)]
                if matches!(file.kind, InputEntryKind::TestFile { .. }) {
                    if extent.size > 0 {
                        self.data
                            .seek(SeekFrom::Current(extent.size as i64 - 1))
                            .await
                            .map_err(io::Error::erase)?;
                        self.data.write_all(&[0]).await?;
                    }
                    offset += extent.size as u64;
                    continue;
                }

                if let InputEntryKind::File(contents) = &file.kind {
                    let chunk_end = (offset + extent.size as u64).min(contents.len() as u64);
                    let range = usize::try_from(offset)
                        .ok()
                        .zip(usize::try_from(chunk_end).ok())
                        .map(|(start, end)| start..end)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "file content range does not fit in memory",
                            )
                        })?;
                    self.data.write_all(&contents[range]).await?;
                    offset += extent.size as u64;
                }
            }
        }

        Ok(())
    }

    async fn patch_moved_directories(
        &mut self,
        root_id: &DirectoryId,
        relocation_refs: &RelocationMap,
    ) -> io::Result<()> {
        let moved = MovedDirectory::collect(self.written_files.get(root_id));
        let directory_end = self.data.stream_position().await.map_err(io::Error::erase)?;

        for moved_dir in moved.iter() {
            for (ty, directory) in moved_dir.entries.iter() {

                if !ty.supports_rrip() {
                    continue;
                }

                let parent = relocation_refs
                    .get(&(moved_dir.logical_parent, *ty))
                    .copied()
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "logical parent extent was not written",
                        )
                    })?;
                self.patch_parent_link(*directory, parent).await?;
            }
        }

        self.data
            .seek(SeekFrom::Start(directory_end))
            .await
            .map_err(io::Error::erase)?;
        Ok(())
    }

    async fn write_path_tables(&mut self) -> io::Result<()> {
        for i in 0..self.entry_types.len() {
            let ty = self.entry_types[i];
            let l_ref = self.write_path_table(ty, EndianType::LittleEndian).await?;
            let m_ref = self.write_path_table(ty, EndianType::BigEndian).await?;
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

    async fn write_path_table(&mut self, ty: EntryType, endian: EndianType) -> io::Result<DirectoryRef> {
        let start = self.data.pad_align_sector().await?;
        PathTableWriter {
            written_files: &self.written_files,
            ty,
            endian,
        }
        .write(&mut self.data).await?;
        let size = self
            .data
            .stream_position()
            .await
            .map_err(io::Error::erase)? as usize
            - (start.0 * self.data.sector_size);
        let _end = self.data.pad_align_sector().await?;
        Ok(DirectoryRef {
            extent: start,
            size,
        })
    }

    /// Writes the partition tables (MBR, GPT, or Hybrid) based on configuration.
    ///
    /// The GPT and Hybrid schemes append 33 sectors of 512 bytes (backup
    /// entry array plus backup header) after the ISO data, padded so that
    /// the image length stays a multiple of the logical sector size and the
    /// backup header occupies the last 512-byte sector. The appended region
    /// lies outside the ISO file-data area but is still counted in
    /// `volume_space_size`, so tools that copy `volume_space_size` logical
    /// sectors preserve the backup GPT, matching xorriso's appended-GPT
    /// practice.
    async fn write_partition_tables(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        match self.ops.partition_scheme() {
            None | Some(PartitionScheme::None) => {
                // No partition table requested - leave the system area empty.
                // Writing an MBR here would cause the kernel to detect a
                // partition table and prevent the ISO from being mounted.
            }
            Some(PartitionScheme::Mbr) => {
                self.write_mbr_boot(end_sector).await?;
            }
            Some(PartitionScheme::Gpt) => {
                self.write_gpt_boot(end_sector).await?;
            }
            Some(PartitionScheme::Hybrid) => {
                self.write_hybrid_boot(end_sector).await?;
            }
        }

        Ok(())
    }

    /// Writes an MBR partition table for BIOS USB boot (isohybrid-style).
    async fn write_mbr_boot(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let sector_count = (end_sector.0 * (self.data.sector_size / 512)) as u32;

        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        let mut mbr = MasterBootRecord::default();
        mbr.with_partition_table(|pt| {
            pt[0] = MbrPartition::new_iso_partition(sector_count, bootable);
        });

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot
            && let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap
        {
            let len = bootstrap.len().min(446);
            mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
        }

        self.data
            .seek(SeekFrom::Start(0))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(bytemuck::bytes_of(&mbr)).await?;

        Ok(())
    }

    /// Resolves the ISO path of the El Torito UEFI image to expose as a GPT
    /// EFI System Partition.
    ///
    /// Uses `HybridBootOptions::efi_boot_partition` when set; otherwise falls
    /// back to the boot image of the single `PlatformId::UEFI` El Torito
    /// section entry, if there is exactly one.
    fn efi_boot_partition_path(&self) -> Option<String> {
        let hybrid = self.ops.features.hybrid_boot.as_ref()?;
        if let Some(path) = &hybrid.efi_boot_partition {
            return Some(path.clone());
        }
        let boot = self.ops.features.el_torito.as_ref()?;
        let mut uefi_entries = boot
            .entries
            .iter()
            .filter(|(section, _)| section.platform == PlatformId::UEFI)
            .map(|(_, entry)| &entry.boot_image_path);
        let first = uefi_entries.next()?;
        if uefi_entries.next().is_some() {
            return None;
        }
        Some(first.clone())
    }

    /// Builds the GPT structures for the image.
    ///
    /// The disk spans the ISO data plus an appended backup-GPT region; the
    /// backup header sits in the last 512-byte sector of the padded image.
    /// The layout is non-overlapping (overlap is rejected by
    /// [`GptDisk::validate`]): a basic-data partition covers the ISO area
    /// before the EFI System Partition, the ESP covers the El Torito UEFI
    /// image exactly, and a second basic-data partition covers any remaining
    /// ISO data after it. Partitions start no earlier than 512-byte LBA 64,
    /// where the ISO volume descriptors begin: tools that convert ISOHYBRID
    /// GPT images to MBR (e.g. `limine bios-install`) embed boot code in
    /// sectors 1..63 and reject partitions starting below sector 63.
    ///
    /// Returns the disk together with the entry indices of the leading
    /// ISO9660 partition and the ESP, for hybrid MBR mirroring.
    /// Total image size in 512-byte sectors for GPT/Hybrid images: the ISO
    /// data plus the appended backup-GPT region, padded to a logical-sector
    /// boundary.
    fn gpt_total_512(&self, end_sector: LogicalSector) -> u64 {
        const BACKUP_GPT_SECTORS: u64 = 33;
        let blocks_per_sector = (self.ops.sector_size / 512) as u64;
        let iso_512 = end_sector.0 as u64 * blocks_per_sector;
        (iso_512 + BACKUP_GPT_SECTORS).div_ceil(blocks_per_sector) * blocks_per_sector
    }

    /// The `volume_space_size` to declare: for GPT/Hybrid images it covers the
    /// whole image including the appended backup-GPT region, so tools that
    /// copy `volume_space_size` logical sectors preserve the backup GPT.
    fn volume_space_sectors(&self, end_sector: LogicalSector) -> u32 {
        match self
            .ops
            .features
            .hybrid_boot
            .as_ref()
            .map(|h| h.partition_scheme)
        {
            Some(PartitionScheme::Gpt) | Some(PartitionScheme::Hybrid) => {
                let blocks_per_sector = (self.ops.sector_size / 512) as u64;
                (self.gpt_total_512(end_sector) / blocks_per_sector) as u32
            }
            _ => end_sector.0 as u32,
        }
    }

    fn build_gpt_disk(
        &self,
        end_sector: LogicalSector,
    ) -> io::Result<(GptDisk, Option<usize>, Option<usize>)> {
        let blocks_per_sector = (self.data.sector_size / 512) as u64;
        let iso_512 = end_sector.0 as u64 * blocks_per_sector;
        let total_512 = self.gpt_total_512(end_sector);

        let mut gpt = GptDisk::new(total_512, 512);
        let disk_guid = generate_guid_from_string(&alloc::format!("disk-{}", self.ops.volume_name));
        gpt.primary_header.disk_guid = disk_guid;
        gpt.backup_header.disk_guid = disk_guid;

        let iso_part_start = self.get_iso_partition_start(&gpt)?;
        let iso_end = iso_512.saturating_sub(1);

        if iso_end <= iso_part_start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "image too small for a GPT partition table",
            ));
        }

        let esp = self.find_esp_partition(blocks_per_sector, iso_part_start, iso_end)?;
        let (iso_index, esp_index) = self.add_gpt_partitions(&mut gpt, esp, iso_part_start, iso_end)?;

        gpt.update_crcs();
        gpt.validate().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid GPT layout"))?;
        Ok((gpt, iso_index, esp_index))
    }

    fn get_iso_partition_start(&self, gpt: &GptDisk) -> io::Result<u64> {
        const ISO_DATA_START_512: u64 = 64;
        let first_usable = gpt.primary_header.first_usable_lba.to_ne();
        Ok(first_usable.max(ISO_DATA_START_512))
    }

    fn find_esp_partition(
        &self,
        blocks_per_sector: u64,
        iso_part_start: u64,
        iso_end: u64,
    ) -> io::Result<Option<(u64, u64)>> {
        let Some(path) = self.efi_boot_partition_path() else { return Ok(None) };

        let dir_ref = self
            .written_files
            .find_file(&path, self.ops.path_separator)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "EFI boot partition image not found in the ISO tree",
                )
            })?;

        let start = dir_ref.extent.0 as u64 * blocks_per_sector;
        let sectors = (dir_ref.size as u64).div_ceil(512).max(1);
        let end = start + sectors - 1;

        if start < iso_part_start || end > iso_end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "EFI boot partition lies outside the ISO data area",
            ));
        }

        Ok(Some((start, end)))
    }

    fn add_gpt_partitions(
        &self,
        gpt: &mut GptDisk,
        esp: Option<(u64, u64)>,
        iso_part_start: u64,
        iso_end: u64,
    ) -> io::Result<(Option<usize>, Option<usize>)> {
        let map_err = |_| io::Error::new(io::ErrorKind::InvalidData, "invalid GPT layout");
        let mut iso_index = None;
        let mut esp_index = None;

        match esp {
            Some((esp_start, esp_end)) => {
                if esp_start > iso_part_start {
                    let mut data = GptPartitionEntry::new(
                        Guid::BASIC_DATA,
                        generate_guid_from_string(&self.ops.volume_name),
                        iso_part_start,
                        esp_start - 1,
                    );
                    data.set_name_ascii(Self::GPT_PARTITION_NAME_ISO);
                    iso_index = Some(gpt.add_partition(data).map_err(map_err)?);
                }

                let mut esp_entry = GptPartitionEntry::new(
                    Guid::EFI_SYSTEM,
                    generate_guid_from_string(&alloc::format!("esp-{}", self.ops.volume_name)),
                    esp_start,
                    esp_end,
                );
                esp_entry.set_name_ascii(Self::GPT_PARTITION_NAME_ESP);
                esp_index = Some(gpt.add_partition(esp_entry).map_err(map_err)?);

                if esp_end < iso_end {
                    let mut tail = GptPartitionEntry::new(
                        Guid::BASIC_DATA,
                        generate_guid_from_string(&alloc::format!("data-{}", self.ops.volume_name)),
                        esp_end + 1,
                        iso_end,
                    );
                    tail.set_name_ascii(Self::GPT_PARTITION_NAME_ISO);
                    gpt.add_partition(tail).map_err(map_err)?;
                }
            }
            None => {
                let mut data = GptPartitionEntry::new(
                    Guid::BASIC_DATA,
                    generate_guid_from_string(&self.ops.volume_name),
                    iso_part_start,
                    iso_end,
                );
                data.set_name_ascii(b"ISO9660");
                iso_index = Some(gpt.add_partition(data).map_err(map_err)?);
            }
        }

        Ok((iso_index, esp_index))
    }

    /// Writes a GPT partition table (protective MBR, primary and backup GPT)
    /// for UEFI boot. See [`Self::write_partition_tables`] for the appended
    /// backup region and [`Self::build_gpt_disk`] for the partition layout.
    async fn write_gpt_boot(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let (gpt, _, _) = self.build_gpt_disk(end_sector)?;
        gpt.write_to(&mut self.data).await.map_err(part_io_error)?;
        Ok(())
    }

    /// Writes a Hybrid MBR + GPT for dual BIOS/UEFI boot.
    ///
    /// The MBR carries a protective entry over the pre-partition gap, mirrors
    /// the leading ISO9660 basic-data partition as type 0x17, and mirrors the
    /// EFI System Partition as type 0xEF.
    async fn write_hybrid_boot(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        let (gpt, iso_index, esp_index) = self.build_gpt_disk(end_sector)?;
        let total_512 = gpt.backup_header.my_lba.to_ne() + 1;

        let mut builder = HybridMbrBuilder::new(total_512).protective_slot(0);
        if let Some(iso_index) = iso_index {
            builder = builder.mirror_partition(iso_index as u32, MbrPartitionType::Iso9660, bootable);
        }
        if let Some(esp_index) = esp_index {
            builder = builder.mirror_partition(
                esp_index as u32,
                MbrPartitionType::EfiSystemPartition,
                false,
            );
        }
        let mut mbr = builder
            .build(&gpt.entries)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid hybrid MBR"))?;

        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot
            && let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap
        {
            let len = bootstrap.len().min(446);
            mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
        }

        gpt.write_to_with_mbr(&mut self.data, &mbr)
            .await
            .map_err(part_io_error)?;

        Ok(())
    }

    async fn update_directory(
        &mut self,
        parent: DirectoryRef,
        directory: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent).await?;
        let mut offset = 0;
        loop {

            if offset >= directory.size as u64 {
                break;
            }

            self.data
                .seek(SeekFrom::Start(start + offset))
                .await
                .map_err(io::Error::erase)?;

            let mut record = DirectoryRecord::parse(&mut self.data).await?;

            if record.is_empty() {
                break;
            }

            if matches!(record.name(), b"\x00" | b"\x01") {
                self.patch_dot_or_dotdot(&mut record, directory, parent, start, offset).await?;
                offset += record.len() as u64;
                continue;
            }

            offset += record.len() as u64;

            if record.flags().contains(FileFlags::DIRECTORY) {
                let record = DirectoryRef {
                    extent: LogicalSector(record.header().extent.read() as usize),
                    size: record.header().data_len.read() as usize,
                };
                self.update_directory(directory, record).await?;
            }
        }

        Ok(())
    }

    async fn patch_dot_or_dotdot(
        &mut self,
        record: &mut DirectoryRecord,
        directory: DirectoryRef,
        parent: DirectoryRef,
        start: u64,
        offset: u64,
    ) -> io::Result<()> {
        let dir_ref = [directory, parent][record.name()[0] as usize];
        let header = record.header_mut();
        header.extent.write(dir_ref.extent.0 as u32);
        header.data_len.write(dir_ref.size as u32);

        self.data
            .seek(SeekFrom::Start(start + offset))
            .await
            .map_err(io::Error::erase)?;
        record.write(&mut self.data).await?;
        Ok(())
    }

    async fn patch_parent_link(
        &mut self,
        directory: DirectoryRef,
        parent: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent).await?;
        let (dot, mut dotdot) = self.read_dot_and_dotdot(start).await?;
        let header_len = dot.header().len as u64;

        self.find_and_patch_pl_entry(&mut dotdot, parent.extent.0 as u32)?;
        self.write_dotdot_at(start, header_len, &dotdot).await?;

        Ok(())
    }

    async fn read_dot_and_dotdot(
        &mut self,
        start: u64,
    ) -> io::Result<(DirectoryRecord, DirectoryRecord)> {
        self.data.seek(SeekFrom::Start(start)).await.map_err(io::Error::erase)?;
        let dot = DirectoryRecord::parse(&mut self.data).await?;

        self.data
            .seek(SeekFrom::Start(start + dot.header().len as u64))
            .await
            .map_err(io::Error::erase)?;
        let dotdot = DirectoryRecord::parse(&mut self.data).await?;

        Ok((dot, dotdot))
    }

    /// Finds and patches the RRIP "PL" (Parent Location) entry in the ".." record.
    ///
    /// Updates the PL entry to point to the logical parent of a relocated directory.
    fn find_and_patch_pl_entry(&self, dotdot: &mut DirectoryRecord, parent_extent: u32) -> io::Result<()> {
        let system_use = dotdot.system_use_mut();
        let mut offset = 0;

        while offset + 4 <= system_use.len() {
            let length = system_use[offset + 2] as usize;
            if length < 4 || offset + length > system_use.len() {
                break;
            }

            if &system_use[offset..offset + 2] == b"PL" && length >= 12 {
                let value = crate::types::U32LsbMsb::new(parent_extent);
                system_use[offset + 4..offset + 12].copy_from_slice(bytemuck::bytes_of(&value));
                return Ok(());
            }

            offset += length;
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "relocated directory is missing its RRIP PL entry",
        ))
    }

    async fn write_dotdot_at(
        &mut self,
        start: u64,
        header_len: u64,
        dotdot: &DirectoryRecord,
    ) -> io::Result<()> {
        self.data
            .seek(SeekFrom::Start(start + header_len))
            .await
            .map_err(io::Error::erase)?;
        dotdot.write(&mut self.data).await?;
        Ok(())
    }

    /// Write records at the extent the planning pass assigned, followed by
    /// the continuation area their CE entries are patched to point at.
    /// Fails if the written layout does not match the plan.
    async fn write_directory_records(
        data: &mut IsoCursor<DATA>,
        sector_size: u64,
        expected: DirectoryRef,
        records: &mut PendingRecords,
    ) -> io::Result<()> {
        let has_overflow = records.has_overflow();
        if has_overflow {
            let ca_sector = expected.extent.0 as u64 + expected.size as u64 / sector_size;
            let mut offset = 0u32;
            for record in records.iter_mut() {
                if record.split.has_overflow() {
                    record.split.patch_ce(ca_sector as u32, offset);
                    offset += record.split.overflow.len() as u32;
                }
            }
        }

        let start = data.pad_align_sector().await?;

        if start != expected.extent {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory extent prediction did not match the written layout",
            ));
        }

        for record in records.iter() {
            let directory_record = DirectoryRecord::new(
                &record.name,
                &record.split.inline,
                record.dir_ref,
                record.flags,
            );
            let position = data.stream_position().await.map_err(io::Error::erase)? as usize;
            let sector_offset = position % data.sector_size;
            let remaining = data.sector_size - sector_offset;
            if directory_record.size() > remaining {
                let padding = vec![0_u8; remaining];
                data.write_all(&padding).await?;
            }
            directory_record.write(&mut *data).await?;
        }

        let end = data.pad_align_sector().await?;
        let size = (end.0 - start.0) * data.sector_size;

        if size != expected.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory size prediction did not match the written layout",
            ));
        }

        if has_overflow {
            for record in records.iter() {
                if record.split.has_overflow() {
                    data.write_all(&record.split.overflow).await?;
                }
            }
        }

        Ok(())
    }
}
} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IsoImage,
        read::PathSeparator,
        write::options::{CreationFeatures, HybridBootOptions},
    };
    use alloc::{string::ToString, vec};
    use core::assert_eq;
    use std::io::Cursor;

    const DIR_NAME_DOT: &[u8] = b"\x00";
    const DIR_NAME_DOTDOT: &[u8] = b"\x01";

    #[test]
    fn should_create_empty_iso_no_boot() {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![],
        };
        let options = IsoFormatOptions {
            volume_name: "EMPTY".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                ..Default::default()
            },
            strict_charset: false,
        };

        let cursor = Cursor::new(vec![0u8; 512]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();

        output
            .seek(SeekFrom::Start(0))
            .expect("Failed to verify ISO image");
        let image = IsoImage::open(output).expect("Failed to parse ISO image");

        let pvd = image.read_pvd().expect("Failed to parse ISO image");
        assert_eq!(pvd.volume_identifier.to_str(), "EMPTY");
        assert_eq!(pvd.system_identifier.to_str(), "SYSTEM");
        assert_eq!(pvd.volume_set_identifier.to_str(), "VOL_SET_ID");
        assert_eq!(pvd.preparer_identifier.to_str(), "PREPARER_ID");
        assert_eq!(pvd.application_identifier.to_str(), "APP_ID");

        assert!(pvd.dir_record.header.is_directory());
        assert_eq!(pvd.dir_record.header.extent.read(), 18);
        assert_eq!(pvd.dir_record.header.data_len.read(), 2048);
        assert_eq!(pvd.dir_record.header.file_identifier_len, 1);
        assert_eq!(pvd.dir_record.header.len, 34);
        assert_eq!(pvd.dir_record.header.volume_sequence_number.read(), 1);
        assert_eq!(pvd.dir_record.header.file_unit_size, 0);
        assert_eq!(pvd.dir_record.header.extended_attr_record, 0);

        assert_eq!(image.info.block_size(), 2048);

        let path_table = image.path_table().path_table;
        assert_eq!(path_table.lpt.0, 19);
        assert_eq!(path_table.mpt.0, 20);
        assert_eq!(path_table.size, 10);

        let susp_info = image.info.susp_info;
        assert_eq!(susp_info.bytes_skipped, 0);
        assert!(!susp_info.detected);
        assert!(!susp_info.rrip_detected);

        assert_eq!(image.root_dirs().iter().count(), 1);

        let root_dir = image.root_dir();
        assert_eq!(root_dir.dir_ref().extent.0, 18);
        assert_eq!(root_dir.dir_ref().size, 2048);
        assert_eq!(
            root_dir.entry_type(),
            EntryType::Level1 {
                supports_lowercase: false,
                supports_rrip: false
            }
        );

        let iso_dir = root_dir.iter(&image);

        assert_eq!(iso_dir.entries().count(), 2);
        let mut entries = iso_dir.entries();
        let current_dir = entries
            .next()
            .unwrap()
            .expect("Failed to parse current dir");
        assert_eq!(current_dir.name(), DIR_NAME_DOT);
        let parent_dir = entries.next().unwrap().expect("Failed to parse parent dir");
        assert_eq!(parent_dir.name(), DIR_NAME_DOTDOT);

        let buffer = image.into_inner().into_inner();
        assert_eq!(
            buffer.len() % 2048,
            0,
            "Image size must be multiple of 2048"
        );
        let sectors_count = buffer.len() / 2048;
        assert_eq!(sectors_count, 21);
    }

    #[test]
    fn should_create_iso_with_file() {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![File::File {
                name: Arc::new("TESTFILE".into()),
                contents: vec![0; 999],
            }],
        };
        let options = IsoFormatOptions {
            volume_name: "EMPTY".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                ..Default::default()
            },
            strict_charset: false,
        };

        let cursor = Cursor::new(vec![0u8; 512]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();

        output
            .seek(SeekFrom::Start(0))
            .expect("Failed to verify ISO image");
        let image = IsoImage::open(output).expect("Failed to parse ISO image");

        let root_dir = image.root_dir();
        let iso_dir = root_dir.iter(&image);

        assert_eq!(iso_dir.entries().count(), 3);
        let mut entries = iso_dir.entries();
        entries.next().unwrap().expect("Failed to parse iso dir");
        entries.next().unwrap().expect("Failed to parse iso dir");
        let file = entries.next().unwrap().expect("Failed to parse iso file");

        assert!(!file.is_directory());
        assert_eq!(file.display_name(), "TESTFILE;1");
        assert_eq!(file.size(), 44); // 33 + 1 + 10 = 44
        assert_eq!(file.total_size(), 999);
        assert!(file.additional_extents.is_empty());
        assert!(file.rrip.is_none());
    }

    #[test]
    fn should_create_iso_with_multi_extent_file() {
        const SIZE: u64 = 4_294_967_296;

        let input = InputTree {
            path_separator: PathSeparator::ForwardSlash,
            entries: vec![InputEntry {
                name: Arc::new("TESTFILE".into()),
                kind: InputEntryKind::TestFile { size: SIZE },
                metadata: InputMetadata::default(),
            }],
        };
        let options = IsoFormatOptions {
            volume_name: "EMPTY".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                ..Default::default()
            },
            strict_charset: false,
        };

        let output = tempfile::tempfile().unwrap();
        let mut output = IsoImageWriter::create(output, input, options).unwrap();

        output
            .seek(SeekFrom::Start(0))
            .expect("Failed to verify ISO image");
        let image = IsoImage::open(output).expect("Failed to parse ISO image");

        let root_dir = image.root_dir();
        let iso_dir = root_dir.iter(&image);

        let mut entries = iso_dir.entries();
        entries.next().unwrap().expect("Failed to parse iso dir");
        entries.next().unwrap().expect("Failed to parse iso dir");
        let mut file = entries.next().unwrap().expect("Failed to parse iso file");

        assert!(!file.is_directory());
        assert_eq!(file.display_name(), "TESTFILE;1");
        assert_eq!(file.size(), 44); // 33 + 1 + 10 = 44
        assert_eq!(file.total_size(), SIZE);
        assert_eq!(file.additional_extents.len(), 1);

        let expected_sector =
            file.header().extent.read() as usize + (4_294_965_248_u64 / 2048) as usize;
        let extent = file.additional_extents.drain(..).next().unwrap();

        assert_eq!(extent.length, 2048);
        assert_eq!(extent.sector.0, expected_sector);

        assert!(file.rrip.is_none());
    }

    #[test]
    fn should_create_iso_with_mbr_boot() {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![File::File {
                name: Arc::new("boot.bin".into()),
                contents: vec![0x55; 2048], // Boot image
            }],
        };
        let options = IsoFormatOptions {
            volume_name: "MBR_TEST".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                hybrid_boot: Some(HybridBootOptions {
                    partition_scheme: PartitionScheme::Mbr,
                    mbr_bootstrap: Some(vec![0x00; 446]), // Bootstrap code
                    bootable: true,
                    efi_boot_partition: None,
                }),
                ..Default::default()
            },
            strict_charset: false,
        };

        let cursor = Cursor::new(vec![0u8; 512]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();

        output.seek(SeekFrom::Start(0)).expect("Failed to seek");

        // Read MBR signature at offset 510-511 (0x55AA)
        let mut sig = [0u8; 2];
        output.seek(SeekFrom::Start(510)).expect("Failed to seek");
        output
            .read_exact(&mut sig)
            .expect("Failed to read MBR signature");
        assert_eq!(sig, [0x55, 0xAA], "MBR signature should be 0x55AA");

        // Read partition table entry at offset 446
        let mut partition = [0u8; 16];
        output.seek(SeekFrom::Start(446)).expect("Failed to seek");
        output
            .read_exact(&mut partition)
            .expect("Failed to read partition entry");

        // Boot indicator should be 0x80 (bootable)
        assert_eq!(partition[0], 0x80, "Partition should be bootable");

        // Partition type should be 0x17 (ISO9660/Hidden NTFS)
        assert_eq!(partition[4], 0x17, "Partition type should be 0x17");
    }

    #[test]
    fn should_create_iso_with_gpt_boot() {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![File::File {
                name: Arc::new("efi.img".into()),
                contents: vec![0xAA; 2048], // EFI boot image
            }],
        };
        let options = IsoFormatOptions {
            volume_name: "GPT_TEST".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                hybrid_boot: Some(HybridBootOptions {
                    partition_scheme: PartitionScheme::Gpt,
                    mbr_bootstrap: None,
                    bootable: false,
                    efi_boot_partition: Some("efi.img".to_string()),
                }),
                ..Default::default()
            },
            strict_charset: false,
        };

        let cursor = Cursor::new(vec![0u8; 512]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();

        output.seek(SeekFrom::Start(0)).expect("Failed to seek");

        // Read protective MBR signature at offset 510-511
        let mut sig = [0u8; 2];
        output.seek(SeekFrom::Start(510)).expect("Failed to seek");
        output
            .read_exact(&mut sig)
            .expect("Failed to read MBR signature");
        assert_eq!(sig, [0x55, 0xAA], "MBR signature should be 0x55AA");

        // Read GPT header at sector 1 (LBA 1)
        let mut header = [0u8; 92];
        output.seek(SeekFrom::Start(512)).expect("Failed to seek"); // 512-byte sectors
        output
            .read_exact(&mut header)
            .expect("Failed to read GPT header");

        // GPT signature is "EFI PART"
        assert_eq!(
            &header[0..8],
            b"EFI PART",
            "GPT header signature should be 'EFI PART'"
        );

        // Check revision (1.0)
        assert_eq!(
            header[8..12],
            [0x00, 0x00, 0x01, 0x00],
            "GPT revision should be 1.0"
        );
    }

    #[test]
    fn should_create_iso_with_hybrid_mbr_gpt_boot() {
        let input = InputFiles {
            path_separator: PathSeparator::ForwardSlash,
            files: vec![
                File::File {
                    name: Arc::new("boot.bin".into()),
                    contents: vec![0x55; 2048], // BIOS boot image
                },
                File::File {
                    name: Arc::new("efi.img".into()),
                    contents: vec![0xAA; 2048], // EFI boot image
                },
            ],
        };
        let options = IsoFormatOptions {
            volume_name: "HYBRID_TEST".to_string(),
            system_id: Some("SYSTEM".to_string()),
            volume_set_id: Some("VOL_SET_ID".to_string()),
            publisher_id: Some("PUBLISHER_ID".to_string()),
            preparer_id: Some("PREPARER_ID".to_string()),
            application_id: Some("APP_ID".to_string()),
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features: CreationFeatures {
                hybrid_boot: Some(HybridBootOptions {
                    partition_scheme: PartitionScheme::Hybrid,
                    mbr_bootstrap: Some(vec![0x00; 446]),
                    bootable: true,
                    efi_boot_partition: Some("efi.img".to_string()),
                }),
                ..Default::default()
            },
            strict_charset: false,
        };

        let cursor = Cursor::new(vec![0u8; 512]);
        let mut output = IsoImageWriter::create(cursor, input, options).unwrap();

        output.seek(SeekFrom::Start(0)).expect("Failed to seek");

        // 1. Check MBR signature
        let mut sig = [0u8; 2];
        output.seek(SeekFrom::Start(510)).expect("Failed to seek");
        output
            .read_exact(&mut sig)
            .expect("Failed to read MBR signature");
        assert_eq!(sig, [0x55, 0xAA], "MBR signature should be 0x55AA");

        // 2. Check partition table entries in MBR
        let mut found_iso = false;
        let mut found_bootable = false;
        let mut found_esp = false;

        for slot in 0..4 {
            let offset = 446 + (slot * 16);
            let mut partition = [0u8; 16];
            output
                .seek(SeekFrom::Start(offset))
                .expect("Failed to seek");
            output
                .read_exact(&mut partition)
                .expect("Failed to read partition");

            // Slot 0 is usually protective MBR (type 0xEE)
            if partition[4] == 0xEE {
                continue; // Skip protective MBR
            }

            // Check for ISO9660 partition (type 0x17)
            if partition[4] == 0x17 {
                found_iso = true;
                if partition[0] == 0x80 {
                    found_bootable = true;
                }
            }

            // Check for EFI System Partition (type 0xEF)
            if partition[4] == 0xEF {
                found_esp = true;
            }
        }

        assert!(found_iso, "ISO9660 partition not found in MBR");
        assert!(found_bootable, "ISO9660 partition should be bootable");
        assert!(found_esp, "EFI System Partition not found in MBR");

        // 3. Check GPT header (dual-boot)
        let mut header = [0u8; 92];
        output.seek(SeekFrom::Start(512)).expect("Failed to seek"); // 512-byte sectors
        output
            .read_exact(&mut header)
            .expect("Failed to read GPT header");
        assert_eq!(
            &header[0..8],
            b"EFI PART",
            "GPT header signature should be 'EFI PART'"
        );

        // Check revision (1.0)
        assert_eq!(
            header[8..12],
            [0x00, 0x00, 0x01, 0x00],
            "GPT revision should be 1.0"
        );

        // 4. Check GPT partition entries
        // Partition type GUID for basic data (ISO9660)
        let basic_data_guid = [
            0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26,
            0x99, 0xC7,
        ];

        // Partition type GUID for EFI System
        let esp_guid = [
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
            0xC9, 0x3B,
        ];

        let mut found_gpt_iso = false;
        let mut found_gpt_esp = false;

        // GPT entries start at LBA 2 (1024 bytes offset in 512-byte sectors)
        for i in 0..4 {
            let offset = 1024 + (i * 128); // Each entry is 128 bytes
            let mut entry = [0u8; 128];
            output
                .seek(SeekFrom::Start(offset))
                .expect("Failed to seek");
            output
                .read_exact(&mut entry)
                .expect("Failed to read GPT partition");

            // Check if entry is non-zero (has a GUID)
            if entry[0..16] == [0u8; 16] {
                continue; // Empty entry
            }

            if entry[0..16] == basic_data_guid {
                found_gpt_iso = true;
            }
            if entry[0..16] == esp_guid {
                found_gpt_esp = true;
            }
        }

        assert!(found_gpt_iso, "ISO9660 partition not found in GPT");
        assert!(found_gpt_esp, "EFI System Partition not found in GPT");
    }
}

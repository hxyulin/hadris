io_transform! {


use crate::read::ContainerInfo;
use crate::types::checksum::verify_object;
use crate::types::container::ContainerSuperblock;
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::container::{CheckpointMapBlock, CheckpointMapping};
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::filesystem::{SYMLINK_XATTR_NAME, XATTR_DATA_EMBEDDED, names_match, stream_size};
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::object_map::{
    BTREE_HASHED, BTREE_PHYSICAL, OMAP_VAL_ENCRYPTED, OMAP_VAL_NOHEADER, ObjectMapBlock,
    VirtualObjectMap,
};
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::{
    FileExtentRecord, FileSystemKey, InodeRecord, ObjectMapKey, ObjectMapValue, OwnedBTreeNode,
    OwnedDirectoryEntryRecord, OwnedEntry, VolumeSuperblock, XattrRecord,
};
use hadris_storage::BlockIndex;
use super::storage::BlockDevice;

/// APFS container reader over a Hadris block device.
#[derive(Debug)]
#[non_exhaustive]
pub struct Container<D> {
    device: D,
    info: ContainerInfo,
    device_block_size: u32,
}

impl<D> Container<D>
where
    D: BlockDevice,
{
    /// Opens an APFS container whose block 0 is at device block 0.
    pub async fn open(device: D) -> hadris_fs::FsResult<Self, D::Error> {
        Self::try_open(device).await.map_err(hadris_fs::MountError::into_error)
    }

    /// Opens a container, returning ownership of the device on failure.
    pub async fn try_open(mut device: D) -> core::result::Result<Self, hadris_fs::MountError<D, D::Error>> {
        match Self::read_info(&mut device).await {
            Ok(info) => {
                let device_block_size = device.block_size().get();
                Ok(Self { device, info, device_block_size })
            }
            Err(error) => Err(hadris_fs::MountError::new(error, device)),
        }
    }

    async fn read_info(device: &mut D) -> hadris_fs::FsResult<ContainerInfo, D::Error> {
        let sector_size = device.block_size().get() as usize;
        if sector_size > 4096 || 4096 % sector_size != 0 {
            return Err(hadris_fs::Error::new(hadris_fs::ErrorKind::Unsupported, "APFS device block size"));
        }
        if device.block_count() < 4096 / sector_size as u64 {
            return Err(crate::ApfsError::InputTooSmall.into());
        }
        let mut header = [0_u8; 4096];
        device.read_blocks(BlockIndex::new(0), &mut header).await?;
        if header[32..36] != crate::types::container::CONTAINER_SUPERBLOCK_MAGIC {
            return Err(hadris_fs::Error::new(hadris_fs::ErrorKind::NotRecognized, "not an APFS container").with_detail(crate::Detail::Magic.code()));
        }
        let superblock = ContainerSuperblock::parse(&header)?;
        if !superblock.block_size.is_power_of_two() || superblock.block_size % sector_size as u32 != 0 {
            return Err(crate::ApfsError::InvalidValue("APFS block size alignment").into());
        }
        if superblock.block_count > device.block_count() / u64::from(superblock.block_size / sector_size as u32) {
            return Err(crate::ApfsError::InvalidValue("APFS container exceeds device").into());
        }
        if superblock.block_size == 4096 {
            verify_object(&header)?;
        } else {
            #[cfg(feature = "alloc")]
            {
                let mut block = alloc::vec![0u8; superblock.block_size as usize];
                device.read_blocks(BlockIndex::new(0), &mut block).await?;
                verify_object(&block)?;
            }
            #[cfg(not(feature = "alloc"))]
            return Err(crate::ApfsError::Unsupported("APFS blocks larger than 4096 bytes without alloc").into());
        }
        Ok(ContainerInfo { superblock })
    }

    /// Returns container metadata parsed during open.
    pub const fn info(&self) -> &ContainerInfo {
        &self.info
    }

    /// Returns the block-zero superblock.
    pub const fn superblock(&self) -> &ContainerSuperblock {
        &self.info.superblock
    }

    /// Reads one APFS container block by APFS physical block number.
    pub async fn read_apfs_block(&mut self, block: u64, buffer: &mut [u8]) -> hadris_fs::FsResult<(), D::Error> {
        if buffer.len() != self.info.superblock.block_size as usize {
            return Err(crate::ApfsError::InvalidValue("APFS block buffer length").into());
        }
        if block >= self.info.superblock.block_count {
            return Err(crate::ApfsError::InvalidValue("APFS block outside container").into());
        }
        let device_block = block
            .checked_mul(u64::from(
                self.info.superblock.block_size / self.device_block_size,
            ))
            .ok_or(crate::ApfsError::AddressOverflow)?;
        self.device
            .read_blocks(BlockIndex::new(device_block), buffer)
            .await

    }

    /// Reads an APFS container block into an owned buffer.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_apfs_block_vec(&mut self, block: u64) -> hadris_fs::FsResult<alloc::vec::Vec<u8>, D::Error> {
        let mut buffer = alloc::vec![0_u8; self.info.superblock.block_size as usize];
        self.read_apfs_block(block, &mut buffer).await?;
        Ok(buffer)
    }

    /// Scans the checkpoint descriptor area for container superblocks, newest first.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn superblocks_sorted(
        &mut self,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<ContainerSuperblock>, D::Error> {
        let mut superblocks = alloc::vec::Vec::new();
        let base = self.info.superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || self.info.superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(crate::ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ).into());
        }
        let count = self.info.superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if self.info.superblock.checkpoint_descriptor_area_length == 0 {
                Ok(superblocks)
            } else {
                Err(crate::ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ).into())
            };
        }
        let start = self.info.superblock.checkpoint_descriptor_area_start_index;
        let length = self.info.superblock.checkpoint_descriptor_area_length;
        if start >= count || length > count || base.checked_add(u64::from(count)).is_none_or(|end| end > self.info.superblock.block_count) {
            return Err(crate::ApfsError::InvalidValue("checkpoint descriptor ring bounds").into());
        }
        for i in 0..length {
            let index = start
                .checked_add(i)
                .ok_or(crate::ApfsError::AddressOverflow)?
                % count;
            let block = base
                .checked_add(u64::from(index))
                .ok_or(crate::ApfsError::AddressOverflow)?;
            let data = self.read_apfs_block_vec(block).await?;
            let object = crate::types::ObjectHeader::parse(&data)?;
            if object.kind() == crate::types::ObjectType::ContainerSuperblock as u16 {
                verify_object(&data)?;
                let checkpoint = ContainerSuperblock::parse(&data)?;
                superblocks.push(checkpoint);
            }
        }
        superblocks.sort_by(|a, b| {
            b.object
                .transaction_identifier
                .cmp(&a.object.transaction_identifier)
        });
        Ok(superblocks)
    }

    /// Returns the newest checkpoint superblock, falling back to block zero when no checkpoint is present.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn latest_superblock(&mut self) -> hadris_fs::FsResult<ContainerSuperblock, D::Error> {
        let checkpoint = self.superblocks_sorted().await?.into_iter().next()
            .unwrap_or_else(|| self.info.superblock.clone());
        if checkpoint.block_size != self.info.superblock.block_size
            || checkpoint.block_count != self.info.superblock.block_count
            || checkpoint.uuid != self.info.superblock.uuid {
            return Err(crate::ApfsError::InvalidValue("checkpoint container geometry or UUID differs").into());
        }
        Ok(checkpoint)
    }

    /// Reads checkpoint map blocks referenced by a superblock.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn checkpoint_map_blocks(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<CheckpointMapBlock>, D::Error> {
        let mut maps = alloc::vec::Vec::new();
        let base = superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(crate::ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ).into());
        }
        let count = superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if superblock.checkpoint_descriptor_area_length == 0 {
                Ok(maps)
            } else {
                Err(crate::ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ).into())
            };
        }
        if superblock.checkpoint_descriptor_area_start_index >= count
            || superblock.checkpoint_descriptor_area_length > count
            || base.checked_add(u64::from(count)).is_none_or(|end| end > self.info.superblock.block_count) {
            return Err(crate::ApfsError::InvalidValue("checkpoint descriptor ring bounds").into());
        }
        for i in 0..superblock.checkpoint_descriptor_area_length {
            let index = superblock
                .checkpoint_descriptor_area_start_index
                .checked_add(i)
                .ok_or(crate::ApfsError::AddressOverflow)?
                % count;
            let block = base
                .checked_add(u64::from(index))
                .ok_or(crate::ApfsError::AddressOverflow)?;
            let data = self.read_apfs_block_vec(block).await?;
            let object = crate::types::ObjectHeader::parse(&data)?;
            if object.kind() == crate::types::ObjectType::CheckpointMap as u16 {
                verify_object(&data)?;
                maps.push(CheckpointMapBlock::parse(&data)?);
            }
        }
        Ok(maps)
    }

    /// Returns flattened checkpoint mappings for a superblock.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn checkpoint_mappings(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<CheckpointMapping>, D::Error> {
        let mut mappings = alloc::vec::Vec::new();
        for map in self.checkpoint_map_blocks(superblock).await? {
            mappings.extend(map.mappings);
        }
        Ok(mappings)
    }

    /// Finds the checkpoint mapping for an ephemeral object identifier.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn find_ephemeral_object_mapping(
        &mut self,
        superblock: &ContainerSuperblock,
        oid: u64,
    ) -> hadris_fs::FsResult<Option<CheckpointMapping>, D::Error> {
        Ok(self
            .checkpoint_mappings(superblock)
            .await?
            .into_iter()
            .find(|mapping| mapping.container_identifier == oid))
    }

    /// Reads the container object map block referenced by a superblock.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<ObjectMapBlock, D::Error> {
        let direct = self
            .read_apfs_block_vec(superblock.object_map_oid)
            .await
            .and_then(|data| {
                verify_object(&data)?;
                Ok(ObjectMapBlock::parse(&data)?)
            });
        let error = match direct {
            Ok(object_map) => return Ok(object_map),
            Err(error) => error,
        };
        if let Some(mapping) = self
            .find_ephemeral_object_mapping(superblock, superblock.object_map_oid)
            .await?
        {
            let data = self.read_apfs_block_vec(mapping.address).await?;
            verify_object(&data)?;
            Ok(ObjectMapBlock::parse(&data)?)
        } else {
            Err(error)
        }
    }

    /// Reads the owned root node of the container object-map B-tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_owned_root_node(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<(ObjectMapBlock, OwnedBTreeNode), D::Error> {
        let object_map = self.object_map(superblock).await?;
        let root = self.read_btree_node(object_map.tree_oid).await?;
        Ok((object_map, root))
    }

    /// Walks the object-map B-tree and returns parsed leaf values.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
        let object_map = self.object_map(superblock).await?;
        self.object_map_values_for(object_map).await
    }

    /// Resolves volume OIDs by walking the object-map B-tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn volume_object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
        Ok(self
            .object_map_values(superblock)
            .await?
            .into_iter()
            .filter(|(key, _)| {
                superblock.volume_oids.contains(&key.oid)
                    || superblock
                        .volume_oids
                        .contains(&(key.oid & 0x0fff_ffff_ffff_ffff))
            })
            .collect())
    }

    /// Reads volume superblocks referenced by the container superblock.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn volume_superblocks(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<VolumeSuperblock>, D::Error> {
        let mut volumes = alloc::vec::Vec::new();
        for (_key, value) in self.volume_object_map_values(superblock).await? {
            let data = self.read_apfs_block_vec(value.address).await?;
            verify_object(&data)?;
            volumes.push(VolumeSuperblock::parse(&data)?);
        }
        Ok(volumes)
    }

    /// Finds the mapping for an object identifier in an object map with the
    /// largest transaction identifier that does not exceed `max_transaction_id`.
    ///
    /// Passing a transaction identifier bound (rather than always taking the
    /// globally newest mapping) is required for correctness: an object map can
    /// contain multiple versions of the same virtual OID (e.g. across
    /// snapshots), and resolving unconditionally to the newest one can return
    /// an object from a later filesystem state than the one being read.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_lookup(
        &mut self,
        object_map: ObjectMapBlock,
        oid: u64,
        max_transaction_id: u64,
    ) -> hadris_fs::FsResult<Option<ObjectMapValue>, D::Error> {
        Ok(self
            .object_map_values_for(object_map)
            .await?
            .into_iter()
            .filter(|(key, _)| {
                (key.oid == oid || (key.oid & 0x0fff_ffff_ffff_ffff) == oid)
                    && key.xid <= max_transaction_id
            })
            .max_by_key(|(key, _)| key.xid)
            .map(|(_, value)| value))
    }

    /// Resolves a virtual object identifier through a volume object map,
    /// bounded to the volume superblock's own transaction identifier.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn resolve_volume_object(
        &mut self,
        volume: &VolumeSuperblock,
        oid: u64,
    ) -> hadris_fs::FsResult<Option<ObjectMapValue>, D::Error> {
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        self.object_map_lookup(object_map, oid, volume.object.transaction_identifier)
            .await
    }

    /// Reads an object map block at a physical APFS block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_at(&mut self, physical_block: u64) -> hadris_fs::FsResult<ObjectMapBlock, D::Error> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        verify_object(&data)?;
        Ok(ObjectMapBlock::parse(&data)?)
    }

    /// Reads the container's space manager summary (free/used block counts).
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn space_manager_summary(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<crate::types::SpaceManagerSummary, D::Error> {
        let data = self.space_manager_block_data(superblock).await?;
        Ok(crate::types::SpaceManagerSummary::parse(&data)?)
    }

    /// Reads the raw bytes of the container's space manager block, resolving
    /// the ephemeral object mapping when the OID isn't a direct physical
    /// block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    async fn space_manager_block_data(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<u8>, D::Error> {
        let direct = match self.read_apfs_block_vec(superblock.space_manager_oid).await {
            Ok(data) => verify_object(&data).map(|_| data).map_err(hadris_fs::Error::from),
            Err(error) => Err(error),
        };
        match direct {
            Ok(data) => Ok(data),
            Err(direct_error) => {
                if let Some(mapping) = self
                    .find_ephemeral_object_mapping(superblock, superblock.space_manager_oid)
                    .await?
                {
                    let data = self.read_apfs_block_vec(mapping.address).await?;
                    verify_object(&data)?;
                    Ok(data)
                } else {
                    Err(direct_error)
                }
            }
        }
    }

    /// Walks the main device's `chunk_info_block_t` blocks and returns all
    /// parsed `chunk_info_t` entries. Only supports the common case where
    /// chunk info addresses are stored inline (no `chunk_info_address_block_t`
    /// indirection).
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn space_manager_chunk_infos(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<crate::types::ChunkInfo>, D::Error> {
        let block_data = self.space_manager_block_data(superblock).await?;
        let summary = crate::types::SpaceManagerSummary::parse(&block_data)?;
        let addresses = summary.main_device_chunk_info_block_addresses(&block_data)?;
        let mut entries = alloc::vec::Vec::new();
        for address in addresses {
            let data = self.read_apfs_block_vec(address).await?;
            verify_object(&data)?;
            entries.extend(crate::types::parse_chunk_info_block(&data)?);
        }
        Ok(entries)
    }

    #[cfg(any(feature = "alloc", feature = "std"))]
    async fn object_map_values_for(
        &mut self,
        object_map: ObjectMapBlock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
        let entries = self.btree_leaf_entries(object_map.tree_oid).await?;
        let mut values = alloc::vec::Vec::new();
        for entry in entries {
            let key = ObjectMapKey {
                oid: crate::types::le_u64(&entry.key, 0)?,
                xid: crate::types::le_u64(&entry.key, 8)?,
            };
            values.push((key, ObjectMapValue::parse(&entry.value)?));
        }
        Ok(values)
    }

    /// Walks a B-tree whose child links are physical block addresses and
    /// returns owned leaf entries in traversal order.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn btree_leaf_entries(
        &mut self,
        root_physical_block: u64,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        self.walk_btree(root_physical_block, 0, 0, None).await
    }

    #[cfg(any(feature = "alloc", feature = "std"))]
    async fn walk_btree(
        &mut self,
        root_physical_block: u64,
        root_flags: u32,
        root_oid: u64,
        virtual_map: Option<&VirtualObjectMap>,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        let root = self
            .read_btree_node_with_flags(root_physical_block, root_flags)
            .await?;
        let info = root.tree_info()?;
        let virtual_children = info.fixed.flags & BTREE_PHYSICAL == 0;
        let mut leaves = alloc::vec::Vec::new();
        let mut visited = alloc::collections::BTreeSet::new();
        visited.insert(root_physical_block);
        let mut stack = alloc::vec![root];
        while let Some(node) = stack.pop() {
            let is_leaf = node.is_leaf()?;
            let entries = node.owned_entries(Some(info))?;
            if is_leaf {
                leaves.extend(entries);
            } else {
                for entry in entries.into_iter().rev() {
                    let mut child_oid = crate::types::le_u64(&entry.value, 0)?;
                    if virtual_children && info.fixed.flags & BTREE_HASHED != 0 {
                        child_oid = root_oid
                            .checked_add(child_oid)
                            .ok_or(crate::ApfsError::AddressOverflow)?;
                    }
                    let (child_block, child_flags) = match virtual_map {
                        Some(map) if virtual_children => {
                            let mapping =
                                map.resolve(child_oid)
                                    .ok_or(crate::ApfsError::InvalidValue(
                                        "B-tree child object not in object map",
                                    ))?;
                            (mapping.address, mapping.flags)
                        }
                        _ => (child_oid, 0),
                    };
                    if !visited.insert(child_block) {
                        return Err(crate::ApfsError::InvalidValue("B-tree node revisited").into());
                    }
                    let child = self
                        .read_btree_node_with_flags(child_block, child_flags)
                        .await?;
                    if node.node()?.level.checked_sub(1) != Some(child.node()?.level) {
                        return Err(crate::ApfsError::InvalidValue("B-tree child level").into());
                    }
                    stack.push(child);
                }
            }
        }
        Ok(leaves)
    }

    /// Loads a volume's object map, bounded to the volume's transaction identifier.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn volume_object_map(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> hadris_fs::FsResult<VirtualObjectMap, D::Error> {
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        let values = self.object_map_values_for(object_map).await?;
        Ok(VirtualObjectMap::new(
            &values,
            volume.object.transaction_identifier,
        ))
    }

    /// Returns owned raw key/value entries from the volume filesystem root tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn filesystem_root_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        let root = self
            .resolve_volume_object(volume, volume.root_tree_oid)
            .await?
            .ok_or(crate::ApfsError::InvalidValue(
                "volume root tree object not found",
            ))?;
        let map = self.volume_object_map(volume).await?;
        self.walk_btree(root.address, root.flags, volume.root_tree_oid, Some(&map))
            .await
    }

    /// Lists owned entries for a directory inode.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<OwnedDirectoryEntryRecord>, D::Error> {
        let mut records = alloc::vec::Vec::new();
        for entry in self.filesystem_root_owned_entries(volume).await? {
            let key = FileSystemKey::parse(&entry.key)?;
            if key.id == directory_id && key.record_type == crate::types::filesystem::FS_TYPE_DIRECTORY_RECORD {
                records.push(OwnedDirectoryEntryRecord::parse(&entry.key, &entry.value)?);
            }
        }
        Ok(records)
    }

    /// Finds an owned entry by name in a directory inode.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
        name: &str,
    ) -> hadris_fs::FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
        let case_insensitive = volume.is_case_insensitive();
        Ok(self
            .directory_owned_entries(volume, directory_id)
            .await?
            .into_iter()
            .find(|entry| names_match(&entry.name, name, case_insensitive)))
    }

    /// Resolves a slash-separated path from the volume root directory.
    /// Handles `.` and `..`, clamps parents at the root, and requires a
    /// directory for intermediate components and trailing slashes.
    /// The volume root is returned with name `/` and inode 2.
    /// Symlinks are returned as entries rather than followed.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn resolve_path(
        &mut self,
        volume: &VolumeSuperblock,
        path: &str,
    ) -> hadris_fs::FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
        let mut directories: alloc::vec::Vec<OwnedDirectoryEntryRecord> = alloc::vec::Vec::new();
        let mut components = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(component) = components.next() {
            if component == "." {
                continue;
            }
            if component == ".." {
                directories.pop();
                continue;
            }
            let parent = directories
                .last()
                .map_or(crate::types::filesystem::INODE_ROOT_DIRECTORY, |entry| {
                    entry.file_id
                });
            let entry = match self
                .directory_owned_entry(volume, parent, component)
                .await?
            {
                Some(entry) => entry,
                None => return Ok(None),
            };
            if (components.peek().is_some() || path.ends_with('/'))
                && entry.file_type() != crate::types::filesystem::DT_DIR
            {
                return Ok(None);
            }
            directories.push(entry);
        }
        Ok(Some(directories.pop().unwrap_or_else(|| {
            OwnedDirectoryEntryRecord {
                parent_id: crate::types::filesystem::INODE_ROOT_DIRECTORY,
                file_id: crate::types::filesystem::INODE_ROOT_DIRECTORY,
                flags: crate::types::filesystem::DT_DIR,
                name: "/".into(),
            }
        })))
    }

    /// Lists owned entries in a volume's root directory when the filesystem root tree is a leaf/root node.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn root_directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<OwnedDirectoryEntryRecord>, D::Error> {
        self.directory_owned_entries(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY)
            .await
    }

    /// Finds an owned root-directory entry by name.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn root_directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        name: &str,
    ) -> hadris_fs::FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
        self.directory_owned_entry(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY, name)
            .await
    }

    /// Finds an inode record by inode identifier in the root filesystem tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn inode_record(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
    ) -> hadris_fs::FsResult<Option<InodeRecord>, D::Error> {
        for entry in self.filesystem_root_owned_entries(volume).await? {
            let key = FileSystemKey::parse(&entry.key)?;
            if key.id == inode && key.record_type == crate::types::filesystem::FS_TYPE_INODE {
                return Ok(Some(InodeRecord::parse(&entry.key, &entry.value)?));
            }
        }
        Ok(None)
    }

    /// Finds file extents for an inode/private data-stream identifier in the root filesystem tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn file_extents(
        &mut self,
        volume: &VolumeSuperblock,
        id: u64,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<FileExtentRecord>, D::Error> {
        let mut extents = alloc::vec::Vec::new();
        for entry in self.filesystem_root_owned_entries(volume).await? {
            let key = FileSystemKey::parse(&entry.key)?;
            if key.id == id && key.record_type == crate::types::filesystem::FS_TYPE_FILE_EXTENT {
                extents.push(FileExtentRecord::parse(&entry.key, &entry.value)?);
            }
        }
        extents.sort_by_key(|extent| extent.logical_address);
        Ok(extents)
    }

    /// Returns the effective file size; see [`stream_size`].
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn file_size(&mut self, volume: &VolumeSuperblock, inode: u64) -> hadris_fs::FsResult<u64, D::Error> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(crate::ApfsError::InvalidValue("inode record not found"))?;
        let extents = self.file_extents(volume, inode.private_id).await?;
        Ok(stream_size(&inode, &extents))
    }

    /// Reads up to `max_bytes` from the start of an uncompressed file into
    /// one buffer, or fails if that much memory cannot be reserved. Holes read
    /// as zeros. Use [`Self::read_extents_at`] to stream large files. Compressed files
    /// return [`hadris_fs::ErrorKind::Unsupported`].
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_file(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
        max_bytes: usize,
    ) -> hadris_fs::FsResult<alloc::vec::Vec<u8>, D::Error> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(crate::ApfsError::InvalidValue("inode record not found"))?;
        if inode.is_compressed() {
            return Err(crate::ApfsError::Unsupported("compressed file data").into());
        }
        let extents = self.file_extents(volume, inode.private_id).await?;
        let size = stream_size(&inode, &extents);
        let len = usize::try_from(size).unwrap_or(usize::MAX).min(max_bytes);
        let mut output = alloc::vec::Vec::new();
        output
            .try_reserve_exact(len)
            .map_err(|_| crate::ApfsError::InvalidValue("file is too large to read into memory"))?;
        output.resize(len, 0);
        let read = self.read_extents_at(&extents, size, 0, &mut output).await?;
        output.truncate(read);
        Ok(output)
    }

    /// Reads file data at `offset` from extents returned by
    /// [`Self::file_extents`] for a file of `size` bytes. Sparse extents and
    /// ranges no extent covers read as zeros. Returns the bytes read, which
    /// is zero at or past `size`.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_extents_at(
        &mut self,
        extents: &[FileExtentRecord],
        size: u64,
        offset: u64,
        buf: &mut [u8],
    ) -> hadris_fs::FsResult<usize, D::Error> {
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        for extent in extents {
            extent.logical_address.checked_add(extent.length).ok_or(crate::ApfsError::AddressOverflow)?;
            if extent.cryptography_id != 0 {
                return Err(crate::ApfsError::Unsupported("encrypted file extents").into());
            }
            if extent.physical_block != 0 {
                let blocks = extent.length.div_ceil(u64::from(self.info.superblock.block_size));
                if extent.physical_block.checked_add(blocks).is_none_or(|end| end > self.info.superblock.block_count) {
                    return Err(crate::ApfsError::InvalidValue("file extent outside container").into());
                }
            }
        }
        let end = size.min(offset.saturating_add(buf.len() as u64));
        let want = (end - offset) as usize;
        buf[..want].fill(0);
        let block_size = u64::from(self.info.superblock.block_size);
        let mut block = alloc::vec![0_u8; block_size as usize];
        for extent in extents {
            let extent_end = extent.logical_address.checked_add(extent.length)
                .ok_or(crate::ApfsError::AddressOverflow)?;
            let mut pos = extent.logical_address.max(offset);
            let stop = extent_end.min(end);
            if extent.physical_block == 0 {
                continue;
            }
            while pos < stop {
                let relative = pos - extent.logical_address;
                let physical = extent
                    .physical_block
                    .checked_add(relative / block_size)
                    .ok_or(crate::ApfsError::AddressOverflow)?;
                self.read_apfs_block(physical, &mut block).await?;
                let within = (relative % block_size) as usize;
                let n = (block_size - within as u64).min(stop - pos) as usize;
                let out = (pos - offset) as usize;
                buf[out..out + n].copy_from_slice(&block[within..within + n]);
                pos += n as u64;
            }
        }
        Ok(want)
    }

    /// Returns a symlink's target from its `com.apple.fs.symlink` attribute,
    /// or `None` when the inode has no such attribute.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn symlink_target(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
    ) -> hadris_fs::FsResult<Option<alloc::string::String>, D::Error> {
        for entry in self.filesystem_root_owned_entries(volume).await? {
            let key = FileSystemKey::parse(&entry.key)?;
            if key.id != inode || key.record_type != crate::types::filesystem::FS_TYPE_XATTR {
                continue;
            }
            let xattr = XattrRecord::parse(&entry.key, &entry.value)?;
            if xattr.id != inode || xattr.name != SYMLINK_XATTR_NAME {
                continue;
            }
            if xattr.flags & XATTR_DATA_EMBEDDED == 0 {
                return Err(crate::ApfsError::Unsupported(
                    "symlink target stored in a data stream",
                ).into());
            }
            let target = xattr.data.strip_suffix(&[0]).unwrap_or(xattr.data);
            let target = core::str::from_utf8(target)
                .map_err(|_| crate::ApfsError::InvalidValue("symlink target UTF-8"))?;
            return Ok(Some(target.into()));
        }
        Ok(None)
    }

    /// Reads an owned B-tree node at a physical APFS block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_btree_node(&mut self, physical_block: u64) -> hadris_fs::FsResult<OwnedBTreeNode, D::Error> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        verify_object(&data)?;
        Ok(OwnedBTreeNode::parse(data)?)
    }

    /// Reads an owned B-tree node using the flags of the object-map entry that
    /// located it. Encrypted nodes are rejected; nodes mapped without an object
    /// header skip the checksum and object-type checks.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_btree_node_with_flags(
        &mut self,
        physical_block: u64,
        omap_flags: u32,
    ) -> hadris_fs::FsResult<OwnedBTreeNode, D::Error> {
        if omap_flags & OMAP_VAL_ENCRYPTED != 0 {
            return Err(crate::ApfsError::Unsupported(
                "encrypted B-tree nodes",
            ).into());
        }
        let data = self.read_apfs_block_vec(physical_block).await?;
        if omap_flags & OMAP_VAL_NOHEADER != 0 {
            return Ok(OwnedBTreeNode::parse_headerless(data)?);
        }
        verify_object(&data)?;
        Ok(OwnedBTreeNode::parse(data)?)
    }

    /// Consumes the reader and returns the wrapped device.
    pub fn into_inner(self) -> D {
        self.device
    }
}

}

//! Asynchronous APFS readers.

use crate::read::ContainerInfo;
use crate::types::checksum::verify_object;
use crate::types::container::ContainerSuperblock;
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::container::{CheckpointMapBlock, CheckpointMapping};
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::object_map::ObjectMapBlock;
#[cfg(any(feature = "alloc", feature = "std"))]
use crate::types::{
    FileExtentRecord, FileSystemKey, InodeRecord, ObjectMapKey, ObjectMapValue, OwnedBTreeNode,
    OwnedDirectoryEntryRecord, OwnedEntry, VolumeSuperblock,
};
use hadris_storage::BlockIndex;
use hadris_storage::r#async::BlockDevice;

/// Asynchronous APFS container reader over a Hadris block device.
#[derive(Debug)]
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
    pub async fn open(mut device: D) -> crate::Result<Self> {
        let sector_size = device.geometry().logical_block_size.get() as usize;
        if sector_size > 4096 || !4096_usize.is_multiple_of(sector_size) {
            return Err(crate::ApfsError::InvalidValue("device block size"));
        }
        let mut header = [0_u8; 4096];
        device
            .read_blocks(BlockIndex(0), &mut header)
            .await
            .map_err(|error| match error {
                hadris_storage::Error::InvalidBufferLength { .. } => {
                    crate::ApfsError::InvalidValue("read buffer length")
                }
                hadris_storage::Error::OutOfBounds { .. } => {
                    crate::ApfsError::InvalidValue("device is smaller than APFS block zero")
                }
                hadris_storage::Error::AddressOverflow => crate::ApfsError::AddressOverflow,
                hadris_storage::Error::InvalidView { .. } => {
                    crate::ApfsError::InvalidValue("partition view")
                }
                hadris_storage::Error::Io(error) => crate::ApfsError::Io(error.kind()),
            })?;
        verify_object(&header)?;
        let superblock = ContainerSuperblock::parse(&header)?;
        if superblock.block_size % sector_size as u32 != 0 {
            return Err(crate::ApfsError::InvalidValue(
                "APFS block size is not device-block aligned",
            ));
        }
        Ok(Self {
            device,
            info: ContainerInfo { superblock },
            device_block_size: sector_size as u32,
        })
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
    pub async fn read_apfs_block(&mut self, block: u64, buffer: &mut [u8]) -> crate::Result<()> {
        if buffer.len() != self.info.superblock.block_size as usize {
            return Err(crate::ApfsError::InvalidValue("APFS block buffer length"));
        }
        let device_block = block
            .checked_mul(u64::from(
                self.info.superblock.block_size / self.device_block_size,
            ))
            .ok_or(crate::ApfsError::AddressOverflow)?;
        self.device
            .read_blocks(BlockIndex(device_block), buffer)
            .await
            .map_err(|error| match error {
                hadris_storage::Error::InvalidBufferLength { .. } => {
                    crate::ApfsError::InvalidValue("read buffer length")
                }
                hadris_storage::Error::OutOfBounds { .. } => {
                    crate::ApfsError::InvalidValue("APFS block is outside the device")
                }
                hadris_storage::Error::AddressOverflow => crate::ApfsError::AddressOverflow,
                hadris_storage::Error::InvalidView { .. } => {
                    crate::ApfsError::InvalidValue("partition view")
                }
                hadris_storage::Error::Io(error) => crate::ApfsError::Io(error.kind()),
            })
    }

    /// Reads an APFS container block into an owned buffer.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_apfs_block_vec(&mut self, block: u64) -> crate::Result<alloc::vec::Vec<u8>> {
        let mut buffer = alloc::vec![0_u8; self.info.superblock.block_size as usize];
        self.read_apfs_block(block, &mut buffer).await?;
        Ok(buffer)
    }

    /// Scans the checkpoint descriptor area for container superblocks, newest first.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn superblocks_sorted(
        &mut self,
    ) -> crate::Result<alloc::vec::Vec<ContainerSuperblock>> {
        let mut superblocks = alloc::vec::Vec::new();
        let base = self.info.superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || self.info.superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(crate::ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ));
        }
        let count = self.info.superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if self.info.superblock.checkpoint_descriptor_area_length == 0 {
                Ok(superblocks)
            } else {
                Err(crate::ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ))
            };
        }
        let start = self.info.superblock.checkpoint_descriptor_area_start_index;
        let length = self.info.superblock.checkpoint_descriptor_area_length;
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
                superblocks.push(ContainerSuperblock::parse(&data)?);
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
    pub async fn latest_superblock(&mut self) -> crate::Result<ContainerSuperblock> {
        Ok(self
            .superblocks_sorted()
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(|| self.info.superblock.clone()))
    }

    /// Reads checkpoint map blocks referenced by a superblock.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn checkpoint_map_blocks(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<alloc::vec::Vec<CheckpointMapBlock>> {
        let mut maps = alloc::vec::Vec::new();
        let base = superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(crate::ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ));
        }
        let count = superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if superblock.checkpoint_descriptor_area_length == 0 {
                Ok(maps)
            } else {
                Err(crate::ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ))
            };
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
    ) -> crate::Result<alloc::vec::Vec<CheckpointMapping>> {
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
    ) -> crate::Result<Option<CheckpointMapping>> {
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
    ) -> crate::Result<ObjectMapBlock> {
        let direct_data = self.read_apfs_block_vec(superblock.object_map_oid).await;
        if let Ok(data) = direct_data
            && verify_object(&data).is_ok()
            && let Ok(object_map) = ObjectMapBlock::parse(&data)
        {
            return Ok(object_map);
        }
        if let Some(mapping) = self
            .find_ephemeral_object_mapping(superblock, superblock.object_map_oid)
            .await?
        {
            let data = self.read_apfs_block_vec(mapping.address).await?;
            verify_object(&data)?;
            ObjectMapBlock::parse(&data)
        } else {
            let data = self.read_apfs_block_vec(superblock.object_map_oid).await?;
            verify_object(&data)?;
            ObjectMapBlock::parse(&data)
        }
    }

    /// Reads the owned root node of the container object-map B-tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_owned_root_node(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<(ObjectMapBlock, OwnedBTreeNode)> {
        let object_map = self.object_map(superblock).await?;
        let root = self.read_btree_node(object_map.tree_oid).await?;
        Ok((object_map, root))
    }

    /// Walks the object-map B-tree and returns parsed leaf values.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>> {
        let object_map = self.object_map(superblock).await?;
        self.object_map_values_for(object_map).await
    }

    /// Resolves volume OIDs by walking the object-map B-tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn volume_object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>> {
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
    ) -> crate::Result<alloc::vec::Vec<VolumeSuperblock>> {
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
    ) -> crate::Result<Option<ObjectMapValue>> {
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
    ) -> crate::Result<Option<ObjectMapValue>> {
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        self.object_map_lookup(object_map, oid, volume.object.transaction_identifier)
            .await
    }

    /// Reads an object map block at a physical APFS block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn object_map_at(&mut self, physical_block: u64) -> crate::Result<ObjectMapBlock> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        verify_object(&data)?;
        ObjectMapBlock::parse(&data)
    }

    /// Reads the container's space manager summary (free/used block counts).
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn space_manager_summary(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<crate::types::SpaceManagerSummary> {
        let data = self.space_manager_block_data(superblock).await?;
        crate::types::SpaceManagerSummary::parse(&data)
    }

    /// Reads the raw bytes of the container's space manager block, resolving
    /// the ephemeral object mapping when the OID isn't a direct physical
    /// block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    async fn space_manager_block_data(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> crate::Result<alloc::vec::Vec<u8>> {
        let direct = match self.read_apfs_block_vec(superblock.space_manager_oid).await {
            Ok(data) => verify_object(&data).map(|_| data),
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
    ) -> crate::Result<alloc::vec::Vec<crate::types::ChunkInfo>> {
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
    ) -> crate::Result<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>> {
        let entries = self.btree_leaf_entries(object_map.tree_oid).await?;
        let mut values = alloc::vec::Vec::new();
        for entry in entries {
            let key = ObjectMapKey {
                oid: u64::from_le_bytes(entry.key[0..8].try_into().expect("omap key oid")),
                xid: u64::from_le_bytes(entry.key[8..16].try_into().expect("omap key xid")),
            };
            values.push((key, ObjectMapValue::parse(&entry.value)?));
        }
        Ok(values)
    }

    /// Walks a B-tree and returns owned leaf entries in traversal order.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn btree_leaf_entries(
        &mut self,
        root_physical_block: u64,
    ) -> crate::Result<alloc::vec::Vec<OwnedEntry>> {
        let root = self.read_btree_node(root_physical_block).await?;
        let info = root.tree_info()?;
        let mut leaves = alloc::vec::Vec::new();
        let mut stack = alloc::vec![root];
        while let Some(node) = stack.pop() {
            let is_leaf = node.is_leaf()?;
            let entries = node.owned_entries(Some(info))?;
            if is_leaf {
                leaves.extend(entries);
            } else {
                for entry in entries.into_iter().rev() {
                    let child_oid =
                        u64::from_le_bytes(entry.value[0..8].try_into().expect("btree child oid"));
                    stack.push(self.read_btree_node(child_oid).await?);
                }
            }
        }
        Ok(leaves)
    }

    /// Returns owned raw key/value entries from the volume filesystem root tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn filesystem_root_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> crate::Result<alloc::vec::Vec<OwnedEntry>> {
        let root = self
            .resolve_volume_object(volume, volume.root_tree_oid)
            .await?
            .ok_or(crate::ApfsError::InvalidValue(
                "volume root tree object not found",
            ))?;
        self.btree_leaf_entries(root.address).await
    }

    /// Lists owned entries for a directory inode.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
    ) -> crate::Result<alloc::vec::Vec<OwnedDirectoryEntryRecord>> {
        Ok(crate::types::filesystem::parse_owned_directory_entries(
            self.filesystem_root_owned_entries(volume).await?,
            directory_id,
        ))
    }

    /// Finds an owned entry by name in a directory inode.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
        name: &str,
    ) -> crate::Result<Option<OwnedDirectoryEntryRecord>> {
        Ok(self
            .directory_owned_entries(volume, directory_id)
            .await?
            .into_iter()
            .find(|entry| entry.name == name))
    }

    /// Resolves a slash-separated path from the volume root directory.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn resolve_path(
        &mut self,
        volume: &VolumeSuperblock,
        path: &str,
    ) -> crate::Result<Option<OwnedDirectoryEntryRecord>> {
        let mut parent = crate::types::filesystem::INODE_ROOT_DIRECTORY;
        let mut current = None;
        for component in path.split('/').filter(|part| !part.is_empty()) {
            let entry = match self
                .directory_owned_entry(volume, parent, component)
                .await?
            {
                Some(entry) => entry,
                None => return Ok(None),
            };
            parent = entry.file_id;
            current = Some(entry);
        }
        Ok(current)
    }

    /// Lists owned entries in a volume's root directory when the filesystem root tree is a leaf/root node.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn root_directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> crate::Result<alloc::vec::Vec<OwnedDirectoryEntryRecord>> {
        self.directory_owned_entries(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY)
            .await
    }

    /// Finds an owned root-directory entry by name.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn root_directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        name: &str,
    ) -> crate::Result<Option<OwnedDirectoryEntryRecord>> {
        self.directory_owned_entry(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY, name)
            .await
    }

    /// Finds an inode record by inode identifier in the root filesystem tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn inode_record(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
    ) -> crate::Result<Option<InodeRecord>> {
        Ok(self
            .filesystem_root_owned_entries(volume)
            .await?
            .into_iter()
            .filter_map(|entry| {
                let key = FileSystemKey::parse(&entry.key).ok()?;
                (key.id == inode && key.record_type == crate::types::filesystem::FS_TYPE_INODE)
                    .then(|| InodeRecord::parse(&entry.key, &entry.value).ok())
                    .flatten()
            })
            .next())
    }

    /// Finds file extents for an inode/private data-stream identifier in the root filesystem tree.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn file_extents(
        &mut self,
        volume: &VolumeSuperblock,
        id: u64,
    ) -> crate::Result<alloc::vec::Vec<FileExtentRecord>> {
        let mut extents: alloc::vec::Vec<FileExtentRecord> = self
            .filesystem_root_owned_entries(volume)
            .await?
            .into_iter()
            .filter_map(|entry| FileExtentRecord::parse(&entry.key, &entry.value).ok())
            .filter(|extent| extent.id == id)
            .collect();
        extents.sort_by_key(|extent| extent.logical_address);
        Ok(extents)
    }

    /// Returns the effective file size: the inode's exact data-stream size when
    /// present, then the inode's uncompressed size when set, otherwise the sum
    /// of the data stream's extent lengths.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn file_size(&mut self, volume: &VolumeSuperblock, inode: u64) -> crate::Result<u64> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(crate::ApfsError::InvalidValue("inode record not found"))?;
        if let Some(size) = inode.data_stream_size {
            return Ok(size);
        }
        if inode.uncompressed_size != 0 {
            return Ok(inode.uncompressed_size);
        }
        Ok(self
            .file_extents(volume, inode.private_id)
            .await?
            .iter()
            .map(|extent| extent.length)
            .sum())
    }

    /// Reads file bytes for a small uncompressed file described by filesystem extents.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_file(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
        max_bytes: usize,
    ) -> crate::Result<alloc::vec::Vec<u8>> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(crate::ApfsError::InvalidValue("inode record not found"))?;
        let known_size = inode.data_stream_size.filter(|size| *size != 0).or({
            if inode.uncompressed_size != 0 {
                Some(inode.uncompressed_size)
            } else {
                None
            }
        });
        let limit = match known_size {
            Some(size) => max_bytes.min(size as usize),
            None => max_bytes,
        };
        let mut output = alloc::vec::Vec::new();
        for extent in self.file_extents(volume, inode.private_id).await? {
            if output.len() >= limit {
                break;
            }
            let mut remaining = extent.length as usize;
            let mut block = extent.physical_block;
            while remaining > 0 && output.len() < limit {
                let data = self.read_apfs_block_vec(block).await?;
                let take = remaining.min(data.len()).min(limit - output.len());
                output.extend_from_slice(&data[..take]);
                remaining -= take;
                block = block
                    .checked_add(1)
                    .ok_or(crate::ApfsError::AddressOverflow)?;
            }
        }
        Ok(output)
    }

    /// Reads an owned B-tree node at a physical APFS block address.
    #[cfg(any(feature = "alloc", feature = "std"))]
    pub async fn read_btree_node(&mut self, physical_block: u64) -> crate::Result<OwnedBTreeNode> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        verify_object(&data)?;
        OwnedBTreeNode::parse(data)
    }

    /// Consumes the reader and returns the wrapped device.
    pub fn into_inner(self) -> D {
        self.device
    }
}

io_transform! {


use crate::read::ContainerInfo;
use crate::{ApfsError, Detail};
use hadris_fs::{Error, ErrorKind, FsResult, MountError};
use crate::types::checksum;
use crate::types::container::ContainerSuperblock;
#[cfg(feature = "alloc")]
use crate::types::container::{CheckpointMapBlock, CheckpointMapping};
#[cfg(feature = "alloc")]
use crate::types::filesystem::{self, SYMLINK_XATTR_NAME, XATTR_DATA_EMBEDDED};
#[cfg(feature = "alloc")]
use crate::types::object_map::{
    BTREE_HASHED, BTREE_PHYSICAL, OMAP_VAL_DELETED, OMAP_VAL_ENCRYPTED, OMAP_VAL_NOHEADER, ObjectMapBlock,
    VirtualObjectMap,
};
#[cfg(feature = "alloc")]
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
    #[cfg(feature = "alloc")]
    zero_error: Option<ApfsError>,
    /// The filesystem trees of the volume queried last, so that keyed
    /// queries do not reload its object map and tree roots.
    #[cfg(feature = "alloc")]
    trees: Option<VolumeTrees>,
    #[cfg(feature = "encryption")]
    unlocked: Option<UnlockedVolume>,
}

#[cfg(feature = "encryption")]
#[derive(Debug)]
struct UnlockedVolume {
    uuid: [u8; 16],
    key: crate::crypto::VolumeKey,
}

/// A B-tree root, read once and kept for keyed descents.
#[cfg(feature = "alloc")]
#[derive(Debug)]
struct BTreeRoot {
    node: OwnedBTreeNode,
    block: u64,
    oid: u64,
    info: crate::types::BTreeInfo,
}

/// The resolved trees of one volume at one transaction.
#[cfg(feature = "alloc")]
#[derive(Debug)]
struct VolumeTrees {
    volume_id: [u8; 16],
    xid: u64,
    map: ObjectMapTree,
    fs: BTreeRoot,
    /// The sealed volume's file extent tree (physical, keyed by data stream).
    fext: Option<BTreeRoot>,
}

/// An object-map tree root, read once and queried by key.
#[cfg(feature = "alloc")]
#[derive(Debug)]
struct ObjectMapTree {
    root: BTreeRoot,
    /// Only mappings at or below this transaction identifier are used.
    max_xid: u64,
}

/// Orders an object-map key (`omap_key_t`) by `oid`, treating a key of `oid`
/// whose transaction identifier is at or below `max_xid` as inside the wanted
/// range. Entries of one object identifier are contiguous and sorted by
/// transaction identifier, so the keys that compare `Equal` are exactly the
/// mappings a lookup may use.
#[cfg(feature = "alloc")]
fn omap_key_order(key: &[u8], oid: u64, max_xid: u64) -> crate::Result<core::cmp::Ordering> {
    let key_oid = crate::types::le_u64(key, 0)?;
    if key_oid != oid {
        return Ok(key_oid.cmp(&oid));
    }
    if crate::types::le_u64(key, 8)? > max_xid {
        return Ok(core::cmp::Ordering::Greater);
    }
    Ok(core::cmp::Ordering::Equal)
}

/// Returns the indices of the index-node children that can hold entries
/// `order` reports as `Equal`. An entry's child spans from its key to the next
/// entry's key, so a child is entered unless it ends before the wanted range
/// or starts after it.
#[cfg(feature = "alloc")]
fn range_children<F>(entries: &[OwnedEntry], order: &F) -> crate::Result<alloc::vec::Vec<usize>>
where
    F: Fn(&[u8]) -> crate::Result<core::cmp::Ordering>,
{
    use core::cmp::Ordering;
    let mut children = alloc::vec::Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if order(&entry.key)? == Ordering::Greater {
            break;
        }
        if let Some(next) = entries.get(i + 1)
            && order(&next.key)? == Ordering::Less
        {
            continue;
        }
        children.push(i);
    }
    Ok(children)
}

/// Orders a filesystem-tree key by its `(object id, record type)` prefix.
#[cfg(feature = "alloc")]
fn fs_key_order(key: &[u8], id: u64, record_type: u8) -> crate::Result<core::cmp::Ordering> {
    let key = FileSystemKey::parse(key)?;
    Ok((key.id, key.record_type).cmp(&(id, record_type)))
}

/// Orders a file-extent-tree key (`fext_tree_key_t`) by its data stream id.
#[cfg(feature = "alloc")]
fn fext_key_order(key: &[u8], id: u64) -> crate::Result<core::cmp::Ordering> {
    Ok(crate::types::le_u64(key, 0)?.cmp(&id))
}

impl<D> Container<D>
where
    D: BlockDevice,
{
    /// Opens an APFS container whose block 0 is at device block 0.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotRecognized`] when block zero has no APFS magic,
    /// or an error for unsupported geometry, invalid container bounds, or a
    /// device failure. The device is retained in [`MountError`] for inspection
    /// or retry; converting that error into [`Error`] drops the device.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn open(mut device: D) -> core::result::Result<Self, MountError<D, D::Error>> {
        match Self::read_info(&mut device).await {
            Ok((info, zero_error)) => {
                let device_block_size = device.block_size().get();
                #[cfg(not(feature = "alloc"))]
                let _ = zero_error;
                Ok(Self { device, info, device_block_size,
                    #[cfg(feature = "alloc")]
                    zero_error,
                    #[cfg(feature = "alloc")]
                    trees: None,
                    #[cfg(feature = "encryption")]
                    unlocked: None,
                })
            }
            Err(error) => Err(MountError::new(error, device)),
        }
    }

    async fn read_info(device: &mut D) -> FsResult<(ContainerInfo, Option<ApfsError>), D::Error> {
        let sector_size = device.block_size().get() as usize;
        if sector_size > 4096 || 4096 % sector_size != 0 {
            return Err(Error::new(ErrorKind::Unsupported, "APFS device block size"));
        }
        if device.block_count() < 4096 / sector_size as u64 {
            return Err(ApfsError::InputTooSmall.into());
        }
        let mut header = [0_u8; 4096];
        device.read_blocks(BlockIndex::new(0), &mut header).await?;
        if header[32..36] != crate::types::container::CONTAINER_SUPERBLOCK_MAGIC {
            return Err(Error::new(ErrorKind::NotRecognized, "not an APFS container").with_detail(Detail::Magic.code()));
        }
        let superblock = ContainerSuperblock::parse(&header)?;
        if !superblock.block_size.is_power_of_two() || superblock.block_size % sector_size as u32 != 0 {
            return Err(ApfsError::InvalidValue("APFS block size alignment").into());
        }
        if superblock.block_count > device.block_count() / u64::from(superblock.block_size / sector_size as u32) {
            return Err(ApfsError::InvalidValue("APFS container exceeds device").into());
        }
        let base = superblock.checkpoint_descriptor_area_block;
        let count = superblock.checkpoint_descriptor_area_block_count;
        if base & (1_u64 << 63) == 0
            && count & (1_u32 << 31) == 0
            && base.checked_add(u64::from(count)).is_none_or(|end| end > superblock.block_count)
        {
            return Err(ApfsError::InvalidValue("checkpoint descriptor area outside container").into());
        }
        let zero_error = if superblock.block_size == 4096 {
            checksum::verify_object(&header).err()
        } else {
            #[cfg(feature = "alloc")]
            {
                let mut block = alloc::vec![0u8; superblock.block_size as usize];
                device.read_blocks(BlockIndex::new(0), &mut block).await?;
                checksum::verify_object(&block).err()
            }
            #[cfg(not(feature = "alloc"))]
            return Err(ApfsError::Unsupported("APFS blocks larger than 4096 bytes without alloc").into());
        };
        #[cfg(not(feature = "alloc"))]
        if let Some(error) = zero_error {
            return Err(error.into());
        }
        Ok((ContainerInfo { superblock }, zero_error))
    }

    /// Returns container metadata parsed during open.
    pub const fn info(&self) -> &ContainerInfo {
        &self.info
    }

    /// Returns the block-zero superblock.
    ///
    /// With `alloc`, opening only requires block zero's magic and geometry,
    /// not its checksum. When the checksum is invalid, this returns the
    /// unverified copy until [`Self::latest_superblock`] selects a verified
    /// checkpoint, which then replaces it.
    pub const fn superblock(&self) -> &ContainerSuperblock {
        &self.info.superblock
    }

    /// Unlocks one software-encrypted, single-key volume with a password.
    ///
    /// `crypto_user` restricts the search to a crypto-user UUID; `None` tries
    /// supported password records. Incorrect credentials return `InvalidInput`
    /// with [`Detail::Credentials`].
    /// Hardware and per-file-key encryption are unsupported. This replaces any
    /// previous unlocked volume, including on failure. Passwords are borrowed
    /// only during this call; retained keys are redacted and wiped on drop.
    /// Keybags are limited to 1 MiB each and total PBKDF2 work to 1,000,000 iterations.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] if the selected volume is absent,
    /// not software encrypted, or credentials fail. Unsupported encryption
    /// returns [`ErrorKind::Unsupported`]. Malformed keybags and device
    /// failures also return errors.
    #[cfg(feature = "encryption")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn unlock_volume(
        &mut self,
        volume: &VolumeSuperblock,
        password: &[u8],
        crypto_user: Option<[u8; 16]>,
    ) -> FsResult<(), D::Error> {
        self.unlocked = None;
        let checkpoint = self.latest_superblock().await?;
        if !self.volume_superblocks(&checkpoint).await?.contains(volume) {
            return Err(ErrorKind::InvalidInput.into());
        }
        self.unlock_selected(&checkpoint, volume, password, crypto_user).await
    }

    #[cfg(feature = "encryption")]
    pub(crate) async fn unlock_selected(
        &mut self,
        checkpoint: &ContainerSuperblock,
        volume: &VolumeSuperblock,
        password: &[u8],
        crypto_user: Option<[u8; 16]>,
    ) -> FsResult<(), D::Error> {
        self.unlocked = None;
        if volume.flags & crate::types::volume::APFS_FS_UNENCRYPTED != 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "APFS volume is not software encrypted"));
        }
        if checkpoint.flags & 4 == 0 || volume.flags & crate::types::volume::APFS_FS_ONEKEY == 0 {
            return Err(ApfsError::Unsupported("APFS hardware or per-file-key encryption").into());
        }
        let container_data = self.read_keybag(checkpoint.keylocker_start, checkpoint.keylocker_blocks, &checkpoint.uuid).await?;
        let container_bag = crate::crypto::Keybag::parse(&container_data, crate::crypto::CONTAINER_KEYBAG)?;
        let (start, blocks) = container_bag.volume_records_range(&volume.volume_id)?;
        let records_data = self.read_keybag(start, blocks, &volume.volume_id).await?;
        let records_bag = crate::crypto::Keybag::parse(&records_data, crate::crypto::VOLUME_KEYBAG)?;
        let key = crate::crypto::unlock_volume_key(&container_bag, &records_bag, &volume.volume_id, crypto_user.as_ref(), password, 1_000_000)?
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "APFS credentials did not unlock the volume").with_detail(Detail::Credentials.code()))?;
        self.verify_unlocked_root(volume, &key).await?;
        self.unlocked = Some(UnlockedVolume { uuid: volume.volume_id, key });
        Ok(())
    }

    #[cfg(feature = "encryption")]
    async fn verify_unlocked_root(&mut self, volume: &VolumeSuperblock, key: &crate::crypto::VolumeKey) -> FsResult<(), D::Error> {
        let root = self.resolve_volume_object(volume, volume.root_tree_oid).await?
            .ok_or(ApfsError::InvalidValue("volume root tree object not found"))?;
        let mut data = self.read_apfs_block_vec(root.address).await?;
        if root.flags & OMAP_VAL_ENCRYPTED != 0 {
            let sector = root.address.checked_mul(u64::from(self.info.superblock.block_size / 512))
                .ok_or(ApfsError::AddressOverflow)?;
            key.decrypt(&mut data, sector)?;
        }
        Self::parse_btree_data(data, root.flags)?;
        Ok(())
    }

    #[cfg(feature = "encryption")]
    pub(crate) fn check_unlocked_volume(&self, volume: &VolumeSuperblock) -> FsResult<(), D::Error> {
        if volume.flags & (crate::types::volume::APFS_FS_ONEKEY | crate::types::volume::APFS_FS_UNENCRYPTED) == crate::types::volume::APFS_FS_ONEKEY
            && self.unlocked.as_ref().is_none_or(|unlocked| unlocked.uuid != volume.volume_id)
        {
            return Err(Error::new(ErrorKind::InvalidInput, "APFS selected volume is not unlocked")
                .with_detail(Detail::Credentials.code()));
        }
        Ok(())
    }

    #[cfg(feature = "encryption")]
    async fn read_keybag(
        &mut self,
        start: u64,
        blocks: u64,
        uuid: &[u8; 16],
    ) -> FsResult<zeroize::Zeroizing<alloc::vec::Vec<u8>>, D::Error> {
        let size = u64::from(self.info.superblock.block_size);
        if blocks == 0 || start == 0 || start.checked_add(blocks).is_none_or(|end| end > self.info.superblock.block_count) {
            return Err(ApfsError::InvalidValue("keybag range outside container").into());
        }
        let bytes = blocks.checked_mul(size).ok_or(ApfsError::AddressOverflow)?;
        if bytes > 1024 * 1024 {
            return Err(ErrorKind::LimitExceeded.into());
        }
        let mut data = zeroize::Zeroizing::new(alloc::vec::Vec::new());
        data.try_reserve_exact(bytes as usize).map_err(|_| ErrorKind::LimitExceeded)?;
        data.resize(bytes as usize, 0);
        for (index, block) in data.chunks_exact_mut(size as usize).enumerate() {
            self.read_apfs_block(start + index as u64, block).await?;
        }
        let first_sector = start.checked_mul(size / 512).ok_or(ApfsError::AddressOverflow)?;
        crate::crypto::decrypt_keybag(&mut data, uuid, first_sector)?;
        Ok(data)
    }

    /// Reads one APFS container block by APFS physical block number.
    ///
    /// # Errors
    ///
    /// Returns a corruption error if the buffer is not exactly one APFS
    /// block or `block` lies outside the container. Device errors propagate
    /// and may leave the buffer partly modified.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(block = block)))]
    pub async fn read_apfs_block(&mut self, block: u64, buffer: &mut [u8]) -> FsResult<(), D::Error> {
        if buffer.len() != self.info.superblock.block_size as usize {
            return Err(ApfsError::InvalidValue("APFS block buffer length").into());
        }
        self.read_apfs_blocks(block, buffer).await
    }

    /// Reads consecutive APFS container blocks starting at `block` into
    /// `buffer`, which holds a whole number of APFS blocks.
    ///
    /// # Errors
    ///
    /// Returns a corruption error for a partial-block buffer or a range
    /// outside the container, or an error if address arithmetic overflows.
    /// Device errors propagate and may leave the buffer partly modified.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(block = block, bytes = buffer.len())))]
    pub async fn read_apfs_blocks(&mut self, block: u64, buffer: &mut [u8]) -> FsResult<(), D::Error> {
        let block_size = self.info.superblock.block_size as usize;
        if buffer.is_empty() || buffer.len() % block_size != 0 {
            return Err(ApfsError::InvalidValue("APFS block buffer length").into());
        }
        let blocks = buffer.len() as u64 / block_size as u64;
        if block >= self.info.superblock.block_count
            || block.checked_add(blocks).is_none_or(|end| end > self.info.superblock.block_count)
        {
            return Err(ApfsError::InvalidValue("APFS block outside container").into());
        }
        let device_block = block
            .checked_mul(u64::from(
                self.info.superblock.block_size / self.device_block_size,
            ))
            .ok_or(ApfsError::AddressOverflow)?;
        self.device
            .read_blocks(BlockIndex::new(device_block), buffer)
            .await
    }

    /// Reads an APFS container block into an owned buffer.
    #[cfg(feature = "alloc")]
    pub async fn read_apfs_block_vec(&mut self, block: u64) -> FsResult<alloc::vec::Vec<u8>, D::Error> {
        let mut buffer = alloc::vec![0_u8; self.info.superblock.block_size as usize];
        self.read_apfs_block(block, &mut buffer).await?;
        Ok(buffer)
    }

    /// Scans every block of the checkpoint descriptor area for valid
    /// container superblocks, newest first. Blocks whose header, checksum or
    /// fields do not parse are skipped, as the mount procedure requires.
    #[cfg(feature = "alloc")]
    pub async fn superblocks_sorted(
        &mut self,
    ) -> FsResult<alloc::vec::Vec<ContainerSuperblock>, D::Error> {
        let mut superblocks = alloc::vec::Vec::new();
        let base = self.info.superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || self.info.superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ).into());
        }
        let count = self.info.superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if self.info.superblock.checkpoint_descriptor_area_length == 0 {
                Ok(superblocks)
            } else {
                Err(ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ).into())
            };
        }
        if base.checked_add(u64::from(count)).is_none_or(|end| end > self.info.superblock.block_count) {
            return Err(ApfsError::InvalidValue("checkpoint descriptor ring bounds").into());
        }
        for index in 0..count {
            let data = self.read_apfs_block_vec(base + u64::from(index)).await?;
            let Ok(object) = crate::types::ObjectHeader::parse(&data) else {
                continue;
            };
            if object.kind() != crate::types::ObjectType::ContainerSuperblock as u16 || checksum::verify_object(&data).is_err() {
                continue;
            }
            if let Ok(checkpoint) = ContainerSuperblock::parse(&data) {
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

    /// Returns the newest checkpoint superblock that verifies.
    ///
    /// Candidates come from [`Self::superblocks_sorted`], then block zero when
    /// its checksum is valid. A candidate is used only when its checkpoint map
    /// blocks, object map and object-map root node verify; otherwise older
    /// ones are tried. While block zero verifies, candidates must match its
    /// block size, block count and UUID; when it does not, they must match
    /// its block size and fit the device. Device errors are returned
    /// immediately. When no candidate verifies, the newest candidate's error,
    /// or block zero's checksum error, is returned.
    #[cfg(feature = "alloc")]
    pub async fn latest_superblock(&mut self) -> FsResult<ContainerSuperblock, D::Error> {
        let zero = self.info.superblock.clone();
        let device_blocks = self.device.block_count() / u64::from(zero.block_size / self.device_block_size);
        let mut first_error = None;
        let mut candidates = self.superblocks_sorted().await?;
        candidates.retain(|checkpoint| {
            checkpoint.block_size == zero.block_size
                && if self.zero_error.is_none() {
                    checkpoint.block_count == zero.block_count && checkpoint.uuid == zero.uuid
                } else {
                    checkpoint.block_count <= device_blocks
                }
        });
        if self.zero_error.is_none() {
            candidates.push(zero);
        }
        for checkpoint in candidates {
            match self.verify_checkpoint(&checkpoint).await {
                Ok(()) => {
                    if self.zero_error.take().is_some() {
                        self.info.superblock = checkpoint.clone();
                    }
                    return Ok(checkpoint);
                }
                Err(error) if error.kind() == ErrorKind::Io => return Err(error),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        Err(first_error.unwrap_or_else(|| {
            self.zero_error.clone().map_or_else(
                || Error::new(ErrorKind::Corrupt, "no APFS checkpoint superblock verifies")
                    .with_detail(Detail::Checksum.code()),
                Error::from,
            )
        }))
    }

    #[cfg(feature = "alloc")]
    async fn verify_checkpoint(&mut self, checkpoint: &ContainerSuperblock) -> FsResult<(), D::Error> {
        let previous = core::mem::replace(&mut self.info.superblock.block_count, checkpoint.block_count);
        let mut result = self.checkpoint_map_blocks(checkpoint).await.map(drop);
        if result.is_ok() {
            result = self.object_map_owned_root_node(checkpoint).await.map(drop);
        }
        self.info.superblock.block_count = previous;
        result
    }

    /// Reads checkpoint map blocks referenced by a superblock.
    #[cfg(feature = "alloc")]
    pub async fn checkpoint_map_blocks(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<CheckpointMapBlock>, D::Error> {
        let mut maps = alloc::vec::Vec::new();
        let base = superblock.checkpoint_descriptor_area_block;
        if base & (1_u64 << 63) != 0
            || superblock.checkpoint_descriptor_area_block_count & (1_u32 << 31) != 0
        {
            return Err(ApfsError::InvalidValue(
                "checkpoint descriptor area is stored as a B-tree",
            ).into());
        }
        let count = superblock.checkpoint_descriptor_area_block_count;
        if count == 0 {
            return if superblock.checkpoint_descriptor_area_length == 0 {
                Ok(maps)
            } else {
                Err(ApfsError::InvalidValue(
                    "zero checkpoint descriptor area block count",
                ).into())
            };
        }
        if superblock.checkpoint_descriptor_area_start_index >= count
            || superblock.checkpoint_descriptor_area_length > count
            || base.checked_add(u64::from(count)).is_none_or(|end| end > self.info.superblock.block_count) {
            return Err(ApfsError::InvalidValue("checkpoint descriptor ring bounds").into());
        }
        for i in 0..superblock.checkpoint_descriptor_area_length {
            let index = superblock
                .checkpoint_descriptor_area_start_index
                .checked_add(i)
                .ok_or(ApfsError::AddressOverflow)?
                % count;
            let block = base
                .checked_add(u64::from(index))
                .ok_or(ApfsError::AddressOverflow)?;
            let data = self.read_apfs_block_vec(block).await?;
            let object = crate::types::ObjectHeader::parse(&data)?;
            if object.kind() == crate::types::ObjectType::CheckpointMap as u16 {
                checksum::verify_object(&data)?;
                maps.push(CheckpointMapBlock::parse(&data)?);
            }
        }
        Ok(maps)
    }

    /// Returns flattened checkpoint mappings for a superblock.
    #[cfg(feature = "alloc")]
    pub async fn checkpoint_mappings(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<CheckpointMapping>, D::Error> {
        let mut mappings = alloc::vec::Vec::new();
        for map in self.checkpoint_map_blocks(superblock).await? {
            mappings.extend(map.mappings);
        }
        Ok(mappings)
    }

    /// Finds the checkpoint mapping for an ephemeral object identifier.
    #[cfg(feature = "alloc")]
    pub async fn find_ephemeral_object_mapping(
        &mut self,
        superblock: &ContainerSuperblock,
        oid: u64,
    ) -> FsResult<Option<CheckpointMapping>, D::Error> {
        Ok(self
            .checkpoint_mappings(superblock)
            .await?
            .into_iter()
            .find(|mapping| mapping.container_identifier == oid))
    }

    /// Reads the container object map block referenced by a superblock.
    #[cfg(feature = "alloc")]
    pub async fn object_map(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<ObjectMapBlock, D::Error> {
        let direct = self
            .read_apfs_block_vec(superblock.object_map_oid)
            .await
            .and_then(|data| {
                checksum::verify_object(&data)?;
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
            checksum::verify_object(&data)?;
            Ok(ObjectMapBlock::parse(&data)?)
        } else {
            Err(error)
        }
    }

    /// Reads the owned root node of the container object-map B-tree.
    #[cfg(feature = "alloc")]
    pub async fn object_map_owned_root_node(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<(ObjectMapBlock, OwnedBTreeNode), D::Error> {
        let object_map = self.object_map(superblock).await?;
        let root = self.read_btree_node(object_map.tree_oid).await?;
        Ok((object_map, root))
    }

    /// Walks the object-map B-tree and returns parsed leaf values.
    #[cfg(feature = "alloc")]
    pub async fn object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
        let object_map = self.object_map(superblock).await?;
        self.object_map_values_for(object_map).await
    }

    /// Resolves volume OIDs by walking the object-map B-tree.
    #[cfg(feature = "alloc")]
    pub async fn volume_object_map_values(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
        Ok(self
            .object_map_values(superblock)
            .await?
            .into_iter()
            .filter(|(key, _)| {
                key.oid != 0 && superblock.volume_oids.contains(&key.oid)
            })
            .collect())
    }

    /// Reads each live volume superblock as of the container superblock's transaction.
    #[cfg(feature = "alloc")]
    pub async fn volume_superblocks(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<VolumeSuperblock>, D::Error> {
        let mut volumes = alloc::vec::Vec::new();
        let object_map = self.object_map(superblock).await?;
        let map = self
            .object_map_tree_root(object_map, superblock.object.transaction_identifier)
            .await?;
        for &oid in superblock.volume_oids.iter().filter(|&&oid| oid != 0) {
            let Some(value) = self.object_map_resolve(&map, oid).await? else {
                continue;
            };
            let data = self.read_apfs_block_vec(value.address).await?;
            checksum::verify_object(&data)?;
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
    /// Deleted mappings return `None` without falling back to older versions.
    #[cfg(feature = "alloc")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn object_map_lookup(
        &mut self,
        object_map: ObjectMapBlock,
        oid: u64,
        max_transaction_id: u64,
    ) -> FsResult<Option<ObjectMapValue>, D::Error> {
        let map = self
            .object_map_tree_root(object_map, max_transaction_id)
            .await?;
        self.object_map_resolve(&map, oid).await
    }

    /// Resolves a virtual object identifier through a volume object map,
    /// bounded to the volume superblock's own transaction identifier.
    #[cfg(feature = "alloc")]
    pub async fn resolve_volume_object(
        &mut self,
        volume: &VolumeSuperblock,
        oid: u64,
    ) -> FsResult<Option<ObjectMapValue>, D::Error> {
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        self.object_map_lookup(object_map, oid, volume.object.transaction_identifier)
            .await
    }

    /// Reads an object map block at a physical APFS block address.
    #[cfg(feature = "alloc")]
    pub async fn object_map_at(&mut self, physical_block: u64) -> FsResult<ObjectMapBlock, D::Error> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        checksum::verify_object(&data)?;
        Ok(ObjectMapBlock::parse(&data)?)
    }

    /// Reads the container's space manager summary (free/used block counts).
    #[cfg(feature = "alloc")]
    pub async fn space_manager_summary(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<crate::types::SpaceManagerSummary, D::Error> {
        let data = self.space_manager_block_data(superblock).await?;
        Ok(crate::types::SpaceManagerSummary::parse(&data)?)
    }

    /// Reads the raw bytes of the container's space manager block, resolving
    /// the ephemeral object mapping when the OID isn't a direct physical
    /// block address.
    #[cfg(feature = "alloc")]
    async fn space_manager_block_data(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<u8>, D::Error> {
        let direct = match self.read_apfs_block_vec(superblock.space_manager_oid).await {
            Ok(data) => checksum::verify_object(&data).map(|_| data).map_err(Error::from),
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
                    checksum::verify_object(&data)?;
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
    #[cfg(feature = "alloc")]
    pub async fn space_manager_chunk_infos(
        &mut self,
        superblock: &ContainerSuperblock,
    ) -> FsResult<alloc::vec::Vec<crate::types::ChunkInfo>, D::Error> {
        let block_data = self.space_manager_block_data(superblock).await?;
        let summary = crate::types::SpaceManagerSummary::parse(&block_data)?;
        let addresses = summary.main_device_chunk_info_block_addresses(&block_data)?;
        let mut entries = alloc::vec::Vec::new();
        for address in addresses {
            let data = self.read_apfs_block_vec(address).await?;
            checksum::verify_object(&data)?;
            entries.extend(crate::types::parse_chunk_info_block(&data)?);
        }
        Ok(entries)
    }

    #[cfg(feature = "alloc")]
    async fn object_map_values_for(
        &mut self,
        object_map: ObjectMapBlock,
    ) -> FsResult<alloc::vec::Vec<(ObjectMapKey, ObjectMapValue)>, D::Error> {
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

    /// Reads the object-map tree rooted at `object_map`, bounded to `max_xid`.
    #[cfg(feature = "alloc")]
    async fn object_map_tree_root(
        &mut self,
        object_map: ObjectMapBlock,
        max_xid: u64,
    ) -> FsResult<ObjectMapTree, D::Error> {
        let node = self.read_btree_node(object_map.tree_oid).await?;
        Ok(ObjectMapTree {
            root: BTreeRoot {
                info: node.tree_info()?,
                node,
                block: object_map.tree_oid,
                // An object-map tree's child links are physical addresses, so
                // no key is rebased against the root's object identifier.
                oid: 0,
            },
            max_xid,
        })
    }

    /// Resolves one object identifier through `map` by key, taking the newest
    /// mapping at or below the map's transaction identifier. A deleted mapping
    /// resolves to `None` without falling back to an older version.
    #[cfg(feature = "alloc")]
    async fn object_map_resolve(
        &mut self,
        map: &ObjectMapTree,
        oid: u64,
    ) -> FsResult<Option<ObjectMapValue>, D::Error> {
        let entries = self.object_map_oid_entries(map, oid).await?;
        let mut newest: Option<(u64, ObjectMapValue)> = None;
        for entry in entries {
            let xid = crate::types::le_u64(&entry.key, 8)?;
            if newest.is_none_or(|(best, _)| xid >= best) {
                newest = Some((xid, ObjectMapValue::parse(&entry.value)?));
            }
        }
        Ok(newest
            .map(|(_, value)| value)
            .filter(|value| value.flags & OMAP_VAL_DELETED == 0))
    }

    /// Collects, in key order, the object-map entries of `oid` whose
    /// transaction identifier is at or below `map.max_xid`.
    ///
    /// An object-map tree stores physical child addresses, so this descent
    /// never resolves a child through an object map. The keyed descent of a
    /// virtual tree does, which is why the two do not call each other.
    #[cfg(feature = "alloc")]
    async fn object_map_oid_entries(
        &mut self,
        map: &ObjectMapTree,
        oid: u64,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        use core::cmp::Ordering;
        let order = |key: &[u8]| omap_key_order(key, oid, map.max_xid);
        let mut found = alloc::vec::Vec::new();
        let mut visited = alloc::collections::BTreeSet::new();
        visited.insert(map.root.block);
        let mut stack = alloc::vec![map.root.node.clone()];
        while let Some(node) = stack.pop() {
            let entries = node.owned_entries(Some(map.root.info))?;
            if node.is_leaf()? {
                for entry in entries {
                    if order(&entry.key)? == Ordering::Equal {
                        found.push(entry);
                    }
                }
                continue;
            }
            let children = range_children(&entries, &order)?;
            // Pushed in reverse so the leftmost child is read first.
            for index in children.into_iter().rev() {
                let child_block = crate::types::le_u64(&entries[index].value, 0)?;
                if !visited.insert(child_block) {
                    return Err(ApfsError::InvalidValue("B-tree node revisited").into());
                }
                let child = self.read_btree_node(child_block).await?;
                if node.node()?.level.checked_sub(1) != Some(child.node()?.level) {
                    return Err(ApfsError::InvalidValue("B-tree child level").into());
                }
                stack.push(child);
            }
        }
        Ok(found)
    }

    /// Walks a B-tree whose child links are physical block addresses and
    /// returns owned leaf entries in traversal order.
    #[cfg(feature = "alloc")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn btree_leaf_entries(
        &mut self,
        root_physical_block: u64,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        self.walk_btree(root_physical_block, 0, 0, None, None).await
    }

    #[cfg(feature = "alloc")]
    async fn walk_btree(
        &mut self,
        root_physical_block: u64,
        root_flags: u32,
        root_oid: u64,
        virtual_map: Option<&ObjectMapTree>,
        volume: Option<&VolumeSuperblock>,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        let root = match volume {
            Some(volume) => self.read_volume_btree_node_with_flags(volume, root_physical_block, root_flags).await?,
            None => self.read_btree_node_with_flags(root_physical_block, root_flags).await?,
        };
        let info = root.tree_info()?;
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
                    let child = self
                        .btree_child(&node, &entry.value, info, root_oid, virtual_map, volume, &mut visited)
                        .await?;
                    stack.push(child);
                }
            }
        }
        Ok(leaves)
    }

    /// Reads the child an index entry's value points at, resolving virtual
    /// children through `virtual_map`, and checks it sits one level below
    /// `parent` and has not been visited before.
    #[cfg(feature = "alloc")]
    #[allow(clippy::too_many_arguments)]
    async fn btree_child(
        &mut self,
        parent: &OwnedBTreeNode,
        value: &[u8],
        info: crate::types::BTreeInfo,
        root_oid: u64,
        virtual_map: Option<&ObjectMapTree>,
        volume: Option<&VolumeSuperblock>,
        visited: &mut alloc::collections::BTreeSet<u64>,
    ) -> FsResult<OwnedBTreeNode, D::Error> {
        let virtual_children = info.fixed.flags & BTREE_PHYSICAL == 0;
        let mut child_oid = crate::types::le_u64(value, 0)?;
        if virtual_children && info.fixed.flags & BTREE_HASHED != 0 {
            child_oid = root_oid
                .checked_add(child_oid)
                .ok_or(ApfsError::AddressOverflow)?;
        }
        let (child_block, child_flags) = match virtual_map {
            Some(map) if virtual_children => {
                let mapping = self.object_map_resolve(map, child_oid).await?.ok_or(
                    ApfsError::InvalidValue("B-tree child object not in object map"),
                )?;
                (mapping.address, mapping.flags)
            }
            _ => (child_oid, 0),
        };
        if !visited.insert(child_block) {
            return Err(ApfsError::InvalidValue("B-tree node revisited").into());
        }
        let child = match volume {
            Some(volume) => self.read_volume_btree_node_with_flags(volume, child_block, child_flags).await?,
            None => self.read_btree_node_with_flags(child_block, child_flags).await?,
        };
        if parent.node()?.level.checked_sub(1) != Some(child.node()?.level) {
            return Err(ApfsError::InvalidValue("B-tree child level").into());
        }
        Ok(child)
    }

    /// Collects, in key order, the leaf entries whose key `order` reports as
    /// `Equal`; `order` places a key relative to the wanted (contiguous) key
    /// range. An index entry's child spans from its key to the next entry's
    /// key, so the descent enters only the children that can hold the range
    /// and reads the paths to it rather than the whole tree.
    #[cfg(feature = "alloc")]
    async fn btree_range<F>(
        &mut self,
        root: &BTreeRoot,
        virtual_map: Option<&ObjectMapTree>,
        volume: Option<&VolumeSuperblock>,
        order: F,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error>
    where
        F: Fn(&[u8]) -> crate::Result<core::cmp::Ordering>,
    {
        use core::cmp::Ordering;
        let mut found = alloc::vec::Vec::new();
        let mut visited = alloc::collections::BTreeSet::new();
        visited.insert(root.block);
        let mut stack = alloc::vec![root.node.clone()];
        while let Some(node) = stack.pop() {
            let entries = node.owned_entries(Some(root.info))?;
            if node.is_leaf()? {
                for entry in entries {
                    if order(&entry.key)? == Ordering::Equal {
                        found.push(entry);
                    }
                }
                continue;
            }
            let children = range_children(&entries, &order)?;
            // Pushed in reverse so the leftmost child is read first.
            for index in children.into_iter().rev() {
                let child = self
                    .btree_child(
                        &node,
                        &entries[index].value,
                        root.info,
                        root.oid,
                        virtual_map,
                        volume,
                        &mut visited,
                    )
                    .await?;
                stack.push(child);
            }
        }
        Ok(found)
    }

    /// Takes the volume's resolved trees out of the cache, loading them when
    /// the cache holds another volume or transaction. Callers put them back.
    #[cfg(feature = "alloc")]
    async fn take_volume_trees(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> FsResult<VolumeTrees, D::Error> {
        #[cfg(feature = "encryption")]
        if self.unlocked.is_some() {
            self.check_unlocked_volume(volume)?;
        }
        let xid = volume.object.transaction_identifier;
        if let Some(trees) = self.trees.take()
            && trees.volume_id == volume.volume_id
            && trees.xid == xid
        {
            return Ok(trees);
        }
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        let map = self
            .object_map_tree_root(object_map, volume.object.transaction_identifier)
            .await?;
        let root = self
            .object_map_resolve(&map, volume.root_tree_oid)
            .await?
            .ok_or(ApfsError::InvalidValue(
                "volume root tree object not found",
            ))?;
        let node = self
            .read_volume_btree_node_with_flags(volume, root.address, root.flags)
            .await?;
        let fs = BTreeRoot {
            info: node.tree_info()?,
            node,
            block: root.address,
            oid: volume.root_tree_oid,
        };
        Ok(VolumeTrees {
            volume_id: volume.volume_id,
            xid,
            map,
            fs,
            // The file extent tree root is read only when a stream needs it.
            fext: None,
        })
    }

    /// Returns, in key order, the filesystem-tree records of one object and
    /// record type (e.g. a directory's entries, or a data stream's extents),
    /// descending the tree by key. The volume's object map and tree root are
    /// loaded on first use and kept for later queries on the same volume.
    #[cfg(feature = "alloc")]
    pub async fn filesystem_records(
        &mut self,
        volume: &VolumeSuperblock,
        id: u64,
        record_type: u8,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        let trees = self.take_volume_trees(volume).await?;
        let found = self
            .btree_range(&trees.fs, Some(&trees.map), Some(volume), |key| {
                fs_key_order(key, id, record_type)
            })
            .await;
        self.trees = Some(trees);
        found
    }

    /// Returns a sealed volume's file extent tree records for one data
    /// stream, or none when the volume has no such tree. The tree root is read
    /// on first use.
    #[cfg(feature = "alloc")]
    pub(crate) async fn sealed_stream_extents(
        &mut self,
        volume: &VolumeSuperblock,
        id: u64,
    ) -> FsResult<alloc::vec::Vec<FileExtentRecord>, D::Error> {
        if !volume.is_sealed() || volume.fext_tree_oid == 0 {
            return Ok(alloc::vec::Vec::new());
        }
        let mut trees = self.take_volume_trees(volume).await?;
        if trees.fext.is_none() {
            let node = self.read_btree_node_with_flags(volume.fext_tree_oid, 0).await?;
            trees.fext = Some(BTreeRoot {
                info: node.tree_info()?,
                node,
                block: volume.fext_tree_oid,
                oid: 0,
            });
        }
        let found = match trees.fext.as_ref() {
            Some(fext) => self.btree_range(fext, None, None, |key| fext_key_order(key, id)).await,
            None => Ok(alloc::vec::Vec::new()),
        };
        self.trees = Some(trees);
        let mut extents = alloc::vec::Vec::new();
        for entry in found? {
            extents.push(FileExtentRecord::parse_fext(&entry.key, &entry.value)?);
        }
        Ok(extents)
    }

    /// Loads a volume's object map, bounded to the volume's transaction identifier.
    #[cfg(feature = "alloc")]
    pub async fn volume_object_map(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> FsResult<VirtualObjectMap, D::Error> {
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        let values = self.object_map_values_for(object_map).await?;
        Ok(VirtualObjectMap::new(
            &values,
            volume.object.transaction_identifier,
        ))
    }

    /// Returns owned raw key/value entries from the volume filesystem root tree.
    #[cfg(feature = "alloc")]
    pub async fn filesystem_root_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> FsResult<alloc::vec::Vec<OwnedEntry>, D::Error> {
        #[cfg(feature = "encryption")]
        if self.unlocked.is_some() {
            self.check_unlocked_volume(volume)?;
        }
        let object_map = self.object_map_at(volume.object_map_oid).await?;
        let map = self
            .object_map_tree_root(object_map, volume.object.transaction_identifier)
            .await?;
        let root = self
            .object_map_resolve(&map, volume.root_tree_oid)
            .await?
            .ok_or(ApfsError::InvalidValue(
                "volume root tree object not found",
            ))?;
        self.walk_btree(root.address, root.flags, volume.root_tree_oid, Some(&map), Some(volume))
            .await
    }

    /// Lists owned entries for a directory inode.
    #[cfg(feature = "alloc")]
    pub async fn directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
    ) -> FsResult<alloc::vec::Vec<OwnedDirectoryEntryRecord>, D::Error> {
        let mut records = alloc::vec::Vec::new();
        let layout = volume.directory_key_layout();
        let entries = self
            .filesystem_records(volume, directory_id, crate::types::filesystem::FS_TYPE_DIRECTORY_RECORD)
            .await?;
        for entry in entries {
            records.push(OwnedDirectoryEntryRecord::parse(&entry.key, &entry.value, layout)?);
        }
        Ok(records)
    }

    /// Finds an owned entry by name in a directory inode.
    #[cfg(feature = "alloc")]
    pub async fn directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        directory_id: u64,
        name: &str,
    ) -> FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
        let comparison = volume.name_comparison();
        Ok(self
            .directory_owned_entries(volume, directory_id)
            .await?
            .into_iter()
            .find(|entry| comparison.matches(&entry.name, name)))
    }

    /// Resolves a slash-separated path from the volume root directory.
    /// Handles `.` and `..`, clamps parents at the root, and requires a
    /// directory for intermediate components and trailing slashes.
    /// The volume root is returned with name `/` and inode 2.
    /// Symlinks are returned as entries rather than followed.
    #[cfg(feature = "alloc")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn resolve_path(
        &mut self,
        volume: &VolumeSuperblock,
        path: &str,
    ) -> FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
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
    #[cfg(feature = "alloc")]
    pub async fn root_directory_owned_entries(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> FsResult<alloc::vec::Vec<OwnedDirectoryEntryRecord>, D::Error> {
        self.directory_owned_entries(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY)
            .await
    }

    /// Finds an owned root-directory entry by name.
    #[cfg(feature = "alloc")]
    pub async fn root_directory_owned_entry(
        &mut self,
        volume: &VolumeSuperblock,
        name: &str,
    ) -> FsResult<Option<OwnedDirectoryEntryRecord>, D::Error> {
        self.directory_owned_entry(volume, crate::types::filesystem::INODE_ROOT_DIRECTORY, name)
            .await
    }

    /// Finds an inode record by inode identifier in the root filesystem tree.
    #[cfg(feature = "alloc")]
    pub async fn inode_record(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
    ) -> FsResult<Option<InodeRecord>, D::Error> {
        let entries = self
            .filesystem_records(volume, inode, crate::types::filesystem::FS_TYPE_INODE)
            .await?;
        match entries.first() {
            Some(entry) => Ok(Some(InodeRecord::parse(&entry.key, &entry.value)?)),
            None => Ok(None),
        }
    }

    /// Finds file extents for an inode/private data-stream identifier in the
    /// root filesystem tree and, on sealed volumes, the file extent tree.
    #[cfg(feature = "alloc")]
    pub async fn file_extents(
        &mut self,
        volume: &VolumeSuperblock,
        id: u64,
    ) -> FsResult<alloc::vec::Vec<FileExtentRecord>, D::Error> {
        let mut extents = alloc::vec::Vec::new();
        let entries = self
            .filesystem_records(volume, id, crate::types::filesystem::FS_TYPE_FILE_EXTENT)
            .await?;
        for entry in entries {
            extents.push(FileExtentRecord::parse(&entry.key, &entry.value)?);
        }
        extents.extend(self.sealed_stream_extents(volume, id).await?);
        extents.sort_by_key(|extent| extent.logical_address);
        Ok(extents)
    }

    /// Returns every extent in a sealed volume's file extent tree, or none
    /// when the volume is not sealed or has no such tree.
    #[cfg(feature = "alloc")]
    pub async fn sealed_file_extents(
        &mut self,
        volume: &VolumeSuperblock,
    ) -> FsResult<alloc::vec::Vec<FileExtentRecord>, D::Error> {
        if !volume.is_sealed() || volume.fext_tree_oid == 0 {
            return Ok(alloc::vec::Vec::new());
        }
        let mut extents = alloc::vec::Vec::new();
        for entry in self.btree_leaf_entries(volume.fext_tree_oid).await? {
            extents.push(FileExtentRecord::parse_fext(&entry.key, &entry.value)?);
        }
        Ok(extents)
    }

    /// Returns the effective file size; see [`filesystem::stream_size`].
    #[cfg(feature = "alloc")]
    pub async fn file_size(&mut self, volume: &VolumeSuperblock, inode: u64) -> FsResult<u64, D::Error> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(ApfsError::InvalidValue("inode record not found"))?;
        let extents = self.file_extents(volume, inode.private_id).await?;
        Ok(filesystem::stream_size(&inode, &extents))
    }

    /// Reads up to `max_bytes` from the start of an uncompressed file into
    /// one buffer, or fails if that much memory cannot be reserved. Holes read
    /// as zeros. Use [`Self::read_extents_at`] to stream large files. Compressed files
    /// return [`ErrorKind::Unsupported`].
    ///
    /// # Errors
    ///
    /// Returns an error if the inode or extents cannot be read, or memory
    /// cannot be reserved. Compressed files and unsupported encryption
    /// return [`ErrorKind::Unsupported`]. Device errors propagate.
    #[cfg(feature = "alloc")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn read_file(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
        max_bytes: usize,
    ) -> FsResult<alloc::vec::Vec<u8>, D::Error> {
        let inode = self
            .inode_record(volume, inode)
            .await?
            .ok_or(ApfsError::InvalidValue("inode record not found"))?;
        if inode.is_compressed() {
            return Err(ApfsError::Unsupported("compressed file data").into());
        }
        let extents = self.file_extents(volume, inode.private_id).await?;
        let size = filesystem::stream_size(&inode, &extents);
        let len = usize::try_from(size).unwrap_or(usize::MAX).min(max_bytes);
        let mut output = alloc::vec::Vec::new();
        output
            .try_reserve_exact(len)
            .map_err(|_| ApfsError::InvalidValue("file is too large to read into memory"))?;
        output.resize(len, 0);
        let read = self.read_volume_extents_at(volume, &extents, size, 0, &mut output).await?;
        output.truncate(read);
        Ok(output)
    }

    /// Reads unencrypted file data at `offset` from extents returned by
    /// [`Self::file_extents`] for a file of `size` bytes. The extents are
    /// sorted by logical address and do not overlap, and only those that cover
    /// `offset` are read. Sparse extents and ranges no extent covers read as
    /// zeros. Returns the bytes read, which is zero at or past `size`.
    /// Encrypted extents require the volume-scoped
    /// [`Self::read_volume_extents_at`] API, even when a volume is unlocked.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid extents or device failures. Encrypted
    /// extents return [`ErrorKind::Unsupported`]. A device failure may leave
    /// `buf` partly modified.
    #[cfg(feature = "alloc")]
    pub async fn read_extents_at(
        &mut self,
        extents: &[FileExtentRecord],
        size: u64,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, D::Error> {
        self.read_extents_inner(extents, size, offset, buf, false).await
    }

    /// Reads extents belonging to `volume`, including software-encrypted data.
    ///
    /// Encrypted extents require that exact volume to be unlocked. Extents must
    /// be obtained from this volume's filesystem tree, sorted by logical
    /// address, and must not overlap. Holes and EOF behave as
    /// in [`Self::read_extents_at`]. The caller's buffer is unchanged on a
    /// credential or extent preflight failure.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid extents, unsupported encryption, or a
    /// volume without matching credentials. Device failures propagate and
    /// may leave `buf` partly modified after preflight succeeds.
    #[cfg(feature = "alloc")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(offset = offset, bytes = buf.len())))]
    pub async fn read_volume_extents_at(
        &mut self,
        volume: &VolumeSuperblock,
        extents: &[FileExtentRecord],
        size: u64,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, D::Error> {
        #[cfg(feature = "encryption")]
        {
            self.check_unlocked_volume(volume)?;
            if extents.iter().any(|extent| extent.cryptography_id != 0)
                && self.unlocked.as_ref().is_none_or(|unlocked| unlocked.uuid != volume.volume_id)
            {
                return Err(Error::new(ErrorKind::InvalidInput, "APFS selected volume is not unlocked")
                    .with_detail(Detail::Credentials.code()));
            }
        }
        #[cfg(not(feature = "encryption"))]
        let _ = volume;
        self.read_extents_inner(extents, size, offset, buf, true).await
    }

    #[cfg(feature = "alloc")]
    async fn read_extents_inner(
        &mut self,
        extents: &[FileExtentRecord],
        size: u64,
        offset: u64,
        buf: &mut [u8],
        volume_scoped: bool,
    ) -> FsResult<usize, D::Error> {
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        self.check_extents(extents, volume_scoped)?;
        self.read_checked_extents_at(extents, size, offset, buf).await
    }

    /// Checks the extents a read is about to use: their logical range must not
    /// overflow, an encrypted one needs this volume's key, and a stored one
    /// must lie inside the container.
    #[cfg(feature = "alloc")]
    fn check_extents(
        &self,
        extents: &[FileExtentRecord],
        volume_scoped: bool,
    ) -> FsResult<(), D::Error> {
        let block_size = u64::from(self.info.superblock.block_size);
        for extent in extents {
            extent.logical_address.checked_add(extent.length).ok_or(ApfsError::AddressOverflow)?;
            if extent.cryptography_id != 0 && (!volume_scoped || !self.has_volume_key()) {
                return Err(ApfsError::Unsupported("encrypted file extents").into());
            }
            if extent.cryptography_id != 0 {
                extent.cryptography_id.checked_add(extent.length.div_ceil(block_size))
                    .and_then(|end| end.checked_mul(u64::from(self.info.superblock.block_size / 512)))
                    .ok_or(ApfsError::AddressOverflow)?;
            }
            if extent.physical_block != 0 {
                let blocks = extent.length.div_ceil(block_size);
                if extent.physical_block.checked_add(blocks).is_none_or(|end| end > self.info.superblock.block_count) {
                    return Err(ApfsError::InvalidValue("file extent outside container").into());
                }
            }
        }
        Ok(())
    }

    /// Reads `offset` of a `size`-byte stream from extents the caller checked
    /// with [`Self::check_extents`], sorted by logical address and without
    /// overlaps. Only the extents that overlap the range are read, whole
    /// blocks at a time. Sparse extents and uncovered ranges read as zeros,
    /// and the returned count is zero at or past `size`.
    #[cfg(feature = "alloc")]
    pub(crate) async fn read_checked_extents_at(
        &mut self,
        extents: &[FileExtentRecord],
        size: u64,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, D::Error> {
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        let end = size.min(offset.saturating_add(buf.len() as u64));
        let want = (end - offset) as usize;
        buf[..want].fill(0);
        let first = extents.partition_point(|extent| {
            extent.logical_address.saturating_add(extent.length) <= offset
        });
        let mut scratch = alloc::vec::Vec::new();
        for extent in &extents[first..] {
            if extent.logical_address >= end {
                break;
            }
            let pos = extent.logical_address.max(offset);
            let stop = extent.logical_address.saturating_add(extent.length).min(end);
            if extent.physical_block == 0 || pos >= stop {
                continue;
            }
            if extent.cryptography_id != 0 && !self.has_volume_key() {
                return Err(ApfsError::Unsupported("encrypted file extents").into());
            }
            #[cfg(feature = "encryption")]
            if extent.cryptography_id != 0 {
                self.read_encrypted_extent_range(extent, pos, stop, offset, buf).await?;
                continue;
            }
            self.read_extent_range(extent, pos, stop, offset, buf, &mut scratch).await?;
        }
        Ok(want)
    }

    /// Reads `[pos, stop)` of one unencrypted extent, whole blocks at a time
    /// and straight into `buf` when the range starts block-aligned. `scratch`
    /// holds partial blocks and is allocated on first use.
    #[cfg(feature = "alloc")]
    async fn read_extent_range(
        &mut self,
        extent: &FileExtentRecord,
        pos: u64,
        stop: u64,
        offset: u64,
        buf: &mut [u8],
        scratch: &mut alloc::vec::Vec<u8>,
    ) -> FsResult<(), D::Error> {
        let block_size = u64::from(self.info.superblock.block_size);
        let mut pos = pos;
        while pos < stop {
            let relative = pos - extent.logical_address;
            let physical = extent
                .physical_block
                .checked_add(relative / block_size)
                .ok_or(ApfsError::AddressOverflow)?;
            let within = (relative % block_size) as usize;
            let out = (pos - offset) as usize;
            let blocks = (stop - pos) / block_size;
            if within == 0 && blocks != 0 {
                let len = (blocks * block_size) as usize;
                self.read_apfs_blocks(physical, &mut buf[out..out + len]).await?;
                pos += len as u64;
                continue;
            }
            if scratch.len() as u64 != block_size {
                scratch.resize(block_size as usize, 0);
            }
            self.read_apfs_block(physical, scratch).await?;
            let n = (block_size - within as u64).min(stop - pos) as usize;
            buf[out..out + n].copy_from_slice(&scratch[within..within + n]);
            pos += n as u64;
        }
        Ok(())
    }

    /// Reads `[pos, stop)` of one encrypted extent, decrypting each block with
    /// the tweak its position implies.
    #[cfg(all(feature = "alloc", feature = "encryption"))]
    async fn read_encrypted_extent_range(
        &mut self,
        extent: &FileExtentRecord,
        pos: u64,
        stop: u64,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<(), D::Error> {
        let block_size = u64::from(self.info.superblock.block_size);
        let mut block = alloc::vec![0_u8; block_size as usize];
        let mut pos = pos;
        while pos < stop {
            let relative = pos - extent.logical_address;
            let physical = extent
                .physical_block
                .checked_add(relative / block_size)
                .ok_or(ApfsError::AddressOverflow)?;
            self.read_apfs_block(physical, &mut block).await?;
            let tweak = extent.cryptography_id.checked_add(relative / block_size)
                .and_then(|block| block.checked_mul(block_size / 512))
                .ok_or(ApfsError::AddressOverflow)?;
            self.unlocked.as_ref().ok_or(ErrorKind::InvalidInput)?.key.decrypt(&mut block, tweak)?;
            let within = (relative % block_size) as usize;
            let n = (block_size - within as u64).min(stop - pos) as usize;
            let out = (pos - offset) as usize;
            buf[out..out + n].copy_from_slice(&block[within..within + n]);
            pos += n as u64;
        }
        Ok(())
    }

    /// Returns a symlink's target from its `com.apple.fs.symlink` attribute,
    /// or `None` when the inode has no such attribute.
    #[cfg(feature = "alloc")]
    pub async fn symlink_target(
        &mut self,
        volume: &VolumeSuperblock,
        inode: u64,
    ) -> FsResult<Option<alloc::string::String>, D::Error> {
        let entries = self
            .filesystem_records(volume, inode, crate::types::filesystem::FS_TYPE_XATTR)
            .await?;
        for entry in entries {
            let xattr = XattrRecord::parse(&entry.key, &entry.value)?;
            if xattr.id != inode || xattr.name != SYMLINK_XATTR_NAME {
                continue;
            }
            if xattr.flags & XATTR_DATA_EMBEDDED == 0 {
                return Err(ApfsError::Unsupported(
                    "symlink target stored in a data stream",
                ).into());
            }
            let target = xattr.data.strip_suffix(&[0]).unwrap_or(xattr.data);
            let target = core::str::from_utf8(target)
                .map_err(|_| ApfsError::InvalidValue("symlink target UTF-8"))?;
            return Ok(Some(target.into()));
        }
        Ok(None)
    }

    /// Reads an owned B-tree node at a physical APFS block address.
    #[cfg(feature = "alloc")]
    pub async fn read_btree_node(&mut self, physical_block: u64) -> FsResult<OwnedBTreeNode, D::Error> {
        let data = self.read_apfs_block_vec(physical_block).await?;
        checksum::verify_object(&data)?;
        Ok(OwnedBTreeNode::parse(data)?)
    }

    /// Reads an owned B-tree node using the flags of the object-map entry that
    /// located it. Encrypted nodes are rejected; nodes mapped without an object
    /// header skip the checksum and object-type checks.
    #[cfg(feature = "alloc")]
    pub async fn read_btree_node_with_flags(
        &mut self,
        physical_block: u64,
        omap_flags: u32,
    ) -> FsResult<OwnedBTreeNode, D::Error> {
        if omap_flags & OMAP_VAL_ENCRYPTED != 0 {
            return Err(ApfsError::Unsupported("encrypted B-tree nodes require volume-scoped reads").into());
        }
        self.read_scoped_btree_node(physical_block, omap_flags).await
    }

    /// Reads a mapped B-tree node belonging to an explicitly identified volume.
    /// Encrypted nodes require that volume's key; mappings must come from its
    /// object map. Checksums and structural validation follow decryption.
    #[cfg(feature = "alloc")]
    pub async fn read_volume_btree_node_with_flags(
        &mut self,
        volume: &VolumeSuperblock,
        physical_block: u64,
        omap_flags: u32,
    ) -> FsResult<OwnedBTreeNode, D::Error> {
        #[cfg(feature = "encryption")]
        if omap_flags & OMAP_VAL_ENCRYPTED != 0
            && self.unlocked.as_ref().is_none_or(|unlocked| unlocked.uuid != volume.volume_id)
        {
            if self.unlocked.is_none() {
                return Err(ApfsError::Unsupported("encrypted B-tree nodes").into());
            }
            return Err(Error::new(ErrorKind::InvalidInput, "APFS selected volume is not unlocked")
                .with_detail(Detail::Credentials.code()));
        }
        #[cfg(not(feature = "encryption"))]
        let _ = volume;
        self.read_scoped_btree_node(physical_block, omap_flags).await
    }

    #[cfg(feature = "alloc")]
    async fn read_scoped_btree_node(
        &mut self,
        physical_block: u64,
        omap_flags: u32,
    ) -> FsResult<OwnedBTreeNode, D::Error> {
        if omap_flags & OMAP_VAL_ENCRYPTED != 0 && !self.has_volume_key() {
            return Err(ApfsError::Unsupported(
                "encrypted B-tree nodes",
            ).into());
        }
        let data = self.read_apfs_block_vec(physical_block).await?;
        #[cfg(feature = "encryption")]
        let data = {
        let mut data = data;
        if omap_flags & OMAP_VAL_ENCRYPTED != 0 {
            let first_sector = physical_block.checked_mul(u64::from(self.info.superblock.block_size / 512))
                .ok_or(ApfsError::AddressOverflow)?;
            self.unlocked.as_ref().ok_or(ErrorKind::InvalidInput)?.key.decrypt(&mut data, first_sector)?;
        }
        data
        };
        Self::parse_btree_data(data, omap_flags)
    }

    #[cfg(feature = "alloc")]
    fn parse_btree_data(data: alloc::vec::Vec<u8>, omap_flags: u32) -> FsResult<OwnedBTreeNode, D::Error> {
        if omap_flags & OMAP_VAL_NOHEADER != 0 {
            return Ok(OwnedBTreeNode::parse_headerless(data)?);
        }
        checksum::verify_object(&data)?;
        Ok(OwnedBTreeNode::parse(data)?)
    }

    #[cfg(feature = "alloc")]
    fn has_volume_key(&self) -> bool {
        #[cfg(feature = "encryption")]
        { self.unlocked.is_some() }
        #[cfg(not(feature = "encryption"))]
        { false }
    }

    /// Consumes the reader and returns the wrapped device.
    pub fn into_inner(self) -> D {
        self.device
    }
}

}

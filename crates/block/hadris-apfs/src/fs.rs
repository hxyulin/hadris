use super::Container;
use super::FileSystem;
use super::storage::BlockDevice;
use crate::types::filesystem::{
    self, FileExtentRecord, FileSystemKey, OwnedDirectoryEntryRecord, XattrRecord, stream_size,
};
use crate::types::{ContainerSuperblock, InodeRecord, VolumeSuperblock};
use crate::{Detail, VolumeSelector};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use hadris_fs::{
    Capabilities, CaseRule, Charset, DateTime, DirCursor, DirEntry, Error, ErrorKind, Field,
    FileType, FsResult, FsStats, Metadata, MountError, MountOptions, Name, NodeId, OpenMode, Owner,
    Permissions, Stored,
};

fn file_type<E>(inode: &InodeRecord) -> FsResult<FileType, E> {
    match inode.mode & 0o170000 {
        0o100000 => Ok(FileType::File),
        0o040000 => Ok(FileType::Dir),
        0o120000 => Ok(FileType::Symlink),
        0o020000 => Ok(FileType::CharDevice),
        0o060000 => Ok(FileType::BlockDevice),
        0o010000 => Ok(FileType::Fifo),
        0o140000 => Ok(FileType::Socket),
        _ => Err(Error::new(ErrorKind::Corrupt, "invalid APFS inode mode")),
    }
}

fn timestamp<E>(nanoseconds: u64) -> FsResult<DateTime, E> {
    Ok(
        DateTime::from_unix_seconds((nanoseconds / 1_000_000_000) as i64)?
            .with_nanoseconds((nanoseconds % 1_000_000_000) as u32)?,
    )
}

fn selection_error<E>(kind: ErrorKind, message: &'static str) -> Error<E> {
    Error::new(kind, message).with_detail(Detail::VolumeSelection.code())
}

#[derive(Debug, Default)]
struct Index {
    inodes: BTreeMap<u64, InodeRecord>,
    directories: BTreeMap<u64, Vec<OwnedDirectoryEntryRecord>>,
    extents: BTreeMap<u64, Vec<FileExtentRecord>>,
    symlinks: BTreeMap<u64, (u16, Vec<u8>)>,
}

impl Index {
    fn parse<E>(entries: Vec<crate::types::OwnedEntry>, limit: Option<usize>) -> FsResult<Self, E> {
        let mut index = Self::default();
        for entry in entries {
            let key = FileSystemKey::parse(&entry.key)?;
            match key.record_type {
                filesystem::FS_TYPE_INODE => {
                    let inode = InodeRecord::parse(&entry.key, &entry.value)?;
                    file_type::<E>(&inode)?;
                    if index.inodes.insert(inode.id, inode).is_some() {
                        return Err(Error::new(ErrorKind::Corrupt, "duplicate APFS inode"));
                    }
                    if limit.is_some_and(|limit| index.inodes.len() > limit) {
                        return Err(ErrorKind::LimitExceeded.into());
                    }
                }
                filesystem::FS_TYPE_DIRECTORY_RECORD => {
                    let record = OwnedDirectoryEntryRecord::parse(&entry.key, &entry.value)?;
                    if record.name == "." || record.name == ".." {
                        continue;
                    }
                    Name::from_bytes(record.name.as_bytes())
                        .check()
                        .map_err(|_| ErrorKind::Corrupt)?;
                    if record.name.len() > 255 {
                        return Err(Error::new(
                            ErrorKind::Corrupt,
                            "APFS name exceeds maximum length",
                        ));
                    }
                    index
                        .directories
                        .entry(record.parent_id)
                        .or_default()
                        .push(record);
                }
                filesystem::FS_TYPE_FILE_EXTENT => {
                    let extent = FileExtentRecord::parse(&entry.key, &entry.value)?;
                    extent
                        .logical_address
                        .checked_add(extent.length)
                        .ok_or(ErrorKind::Corrupt)?;
                    index.extents.entry(extent.id).or_default().push(extent);
                }
                filesystem::FS_TYPE_XATTR => {
                    let xattr = XattrRecord::parse(&entry.key, &entry.value)?;
                    if xattr.name == filesystem::SYMLINK_XATTR_NAME
                        && index
                            .symlinks
                            .insert(xattr.id, (xattr.flags, xattr.data.to_vec()))
                            .is_some()
                    {
                        return Err(Error::new(
                            ErrorKind::Corrupt,
                            "duplicate APFS symlink target",
                        ));
                    }
                }
                _ => {}
            }
        }
        for extents in index.extents.values_mut() {
            extents.sort_by_key(|extent| extent.logical_address);
            let mut end = 0;
            for extent in extents {
                if extent.logical_address < end {
                    return Err(Error::new(ErrorKind::Corrupt, "overlapping APFS extents"));
                }
                end = extent
                    .logical_address
                    .checked_add(extent.length)
                    .ok_or(ErrorKind::Corrupt)?;
            }
        }
        Ok(index)
    }

    fn validate<E>(
        &self,
        volume: &VolumeSuperblock,
        block_size: u32,
        block_count: u64,
    ) -> FsResult<(), E> {
        for (&parent, entries) in &self.directories {
            if parent != 1 {
                let parent_inode = self.inodes.get(&parent).ok_or_else(|| {
                    Error::new(ErrorKind::Corrupt, "APFS directory parent missing")
                })?;
                if !file_type::<E>(parent_inode)?.is_dir() {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "APFS directory parent is not a directory",
                    ));
                }
            } else if entries
                .iter()
                .any(|entry| !matches!(entry.file_id, filesystem::INODE_ROOT_DIRECTORY | 3))
            {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    "invalid APFS root-parent entry",
                ));
            }
            let mut names = alloc::collections::BTreeSet::new();
            for entry in entries {
                let name = if volume.is_case_insensitive() {
                    entry
                        .name
                        .chars()
                        .flat_map(char::to_lowercase)
                        .collect::<alloc::string::String>()
                } else {
                    entry.name.clone()
                };
                if !names.insert(name) {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "duplicate APFS directory name",
                    ));
                }
                let inode = self.inodes.get(&entry.file_id).ok_or_else(|| {
                    Error::new(ErrorKind::Corrupt, "APFS directory entry inode missing")
                })?;
                let kind = file_type::<E>(inode)?;
                let expected = match kind {
                    FileType::File => 8,
                    FileType::Dir => 4,
                    FileType::Symlink => 10,
                    FileType::CharDevice => 2,
                    FileType::BlockDevice => 6,
                    FileType::Fifo => 1,
                    FileType::Socket => 12,
                    _ => return Err(ErrorKind::Corrupt.into()),
                };
                if entry.file_type() != 0 && entry.file_type() != expected {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "APFS directory entry type disagrees with inode",
                    ));
                }
                if kind.is_dir() && (inode.parent_id != parent || inode.id == parent) {
                    return Err(Error::new(
                        ErrorKind::Corrupt,
                        "APFS directory parent disagrees with inode",
                    ));
                }
            }
        }
        for extent in self.extents.values().flatten() {
            if extent.physical_block == 0 {
                continue;
            }
            let blocks = extent.length.div_ceil(u64::from(block_size));
            if extent
                .physical_block
                .checked_add(blocks)
                .is_none_or(|end| end > block_count)
            {
                return Err(Error::new(
                    ErrorKind::Corrupt,
                    "APFS file extent outside container",
                ));
            }
        }
        Ok(())
    }

    fn target<E>(&self, inode: u64) -> FsResult<&[u8], E> {
        let (flags, data) = self
            .symlinks
            .get(&inode)
            .ok_or_else(|| Error::new(ErrorKind::Corrupt, "APFS symlink target missing"))?;
        if flags & filesystem::XATTR_DATA_EMBEDDED == 0 {
            return Err(
                crate::ApfsError::Unsupported("symlink target stored in a data stream").into(),
            );
        }
        let target = data.strip_suffix(&[0]).unwrap_or(data);
        core::str::from_utf8(target).map_err(|_| ErrorKind::Corrupt)?;
        if target.contains(&0) {
            return Err(Error::new(
                ErrorKind::Corrupt,
                "APFS symlink target contains NUL",
            ));
        }
        Ok(target)
    }

    fn extents(&self, id: u64) -> &[FileExtentRecord] {
        self.extents.get(&id).map_or(&[], Vec::as_slice)
    }
}

io_transform! {

/// Read-only APFS volume driver implementing the shared filesystem contract.
///
/// Node identifiers are stable APFS inode identifiers. The driver owns one
/// container and selects one volume from its newest checkpoint. It allocates
/// metadata once at mount, but reads file data with one APFS block of scratch
/// space regardless of file length. Compression, encryption and snapshots
/// remain unsupported.
#[derive(Debug)]
#[non_exhaustive]
pub struct ApfsFs<D> {
    container: Container<D>,
    checkpoint: ContainerSuperblock,
    volume: VolumeSuperblock,
    index: Index,
}

impl<D: BlockDevice> ApfsFs<D> {
    /// Mounts the sole volume in the container's newest checkpoint.
    ///
    /// Containers with no volumes fail with `NotFound`; multiple volumes
    /// require [`Self::mount_volume`] and fail with `InvalidInput`.
    /// Mount failures return the device for inspection or retry.
    pub async fn mount(device: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, None).await
    }

    /// Mounts exactly one volume selected by index, object identifier, UUID or
    /// exact name. Missing selectors fail with `NotFound`; duplicate matches
    /// fail with `InvalidInput`. Names are case sensitive for selection.
    pub async fn mount_volume(
        device: D,
        options: MountOptions,
        selector: VolumeSelector<'_>,
    ) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, Some(selector)).await
    }

    async fn mount_selected(
        device: D,
        options: MountOptions,
        selector: Option<VolumeSelector<'_>>,
    ) -> Result<Self, MountError<D, D::Error>> {
        let mut container = Container::try_open(device).await?;
        match Self::select_volume(&mut container, selector, options.node_limit()).await {
            Ok((checkpoint, volume, index)) => Ok(Self { container, checkpoint, volume, index }),
            Err(error) => Err(MountError::new(error, container.into_inner())),
        }
    }

    async fn select_volume(
        container: &mut Container<D>,
        selector: Option<VolumeSelector<'_>>,
        limit: Option<usize>,
    ) -> FsResult<(ContainerSuperblock, VolumeSuperblock, Index), D::Error> {
        let checkpoint = container.latest_superblock().await?;
        let volumes = container.volume_superblocks(&checkpoint).await?;
        let mut selected = None;
        for volume in volumes {
            let matches = match selector {
                None => true,
                Some(VolumeSelector::Index(index)) => volume.fs_index == index,
                Some(VolumeSelector::ObjectId(oid)) => volume.object.identifier == oid,
                Some(VolumeSelector::Uuid(uuid)) => volume.volume_id == uuid,
                Some(VolumeSelector::Name(name)) => volume.name()? == name,
            };
            if matches {
                if selected.is_some() {
                    return Err(selection_error(ErrorKind::InvalidInput, "APFS volume selection is ambiguous"));
                }
                selected = Some(volume);
            }
        }
        let volume = selected.ok_or_else(|| selection_error(ErrorKind::NotFound, "APFS volume not found"))?;
        let index = Index::parse(container.filesystem_root_owned_entries(&volume).await?, limit)?;
        index.validate::<D::Error>(&volume, checkpoint.block_size, checkpoint.block_count)?;
        let root = index.inodes.get(&crate::types::filesystem::INODE_ROOT_DIRECTORY)
            .ok_or_else(|| Error::new(ErrorKind::Corrupt, "APFS root inode missing"))?;
        if !file_type::<D::Error>(root)?.is_dir() {
            return Err(Error::new(ErrorKind::Corrupt, "APFS root is not a directory"));
        }
        Ok((checkpoint, volume, index))
    }

    /// The selected volume superblock, including its UUID, name and slot.
    pub const fn volume_superblock(&self) -> &VolumeSuperblock {
        &self.volume
    }

    /// The checkpoint from which the selected volume was opened.
    pub const fn checkpoint(&self) -> &ContainerSuperblock {
        &self.checkpoint
    }

    /// The container reader for native metadata inspection.
    pub const fn container(&self) -> &Container<D> {
        &self.container
    }

    /// Borrows the container reader for native metadata inspection.
    pub fn container_mut(&mut self) -> &mut Container<D> {
        &mut self.container
    }

    /// Returns the container for continued native inspection.
    pub fn into_container(self) -> Container<D> {
        self.container
    }

    /// Returns the owned device without issuing I/O.
    pub fn into_inner(self) -> D {
        self.container.into_inner()
    }

    /// Unmounts the read-only driver and returns its device.
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.container.into_inner())
    }

    async fn inode(&mut self, node: NodeId) -> FsResult<InodeRecord, D::Error> {
        self.index.inodes.get(&node.get()).copied().ok_or_else(|| ErrorKind::InvalidHandle.into())
    }

    async fn directory(&mut self, node: NodeId) -> FsResult<InodeRecord, D::Error> {
        let inode = self.inode(node).await?;
        if !file_type::<D::Error>(&inode)?.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(inode)
    }

    async fn metadata(&mut self, inode: &InodeRecord) -> FsResult<Metadata, D::Error> {
        let kind = file_type::<D::Error>(inode)?;
        let mut metadata = Metadata::new(kind, Permissions::new(u32::from(inode.mode & 0o7777)))
            .with_owner(Owner::new(inode.owner, inode.group))
            .with_created(timestamp::<D::Error>(inode.create_time_ns)?)
            .with_modified(timestamp::<D::Error>(inode.modification_time_ns)?)
            .with_changed(timestamp::<D::Error>(inode.change_time_ns)?)
            .with_accessed(timestamp::<D::Error>(inode.access_time_ns)?)
            .with_nlink(if kind.is_dir() { 1 } else { u64::try_from(inode.link_or_child_count).map_err(|_| ErrorKind::Corrupt)?.max(1) });
        if kind.is_file() {
            let extents = self.index.extents(inode.private_id);
            let allocated = extents.iter().filter(|extent| extent.physical_block != 0)
                .try_fold(0u64, |total, extent| total.checked_add(extent.length))
                .ok_or(ErrorKind::Corrupt)?;
            metadata = metadata.with_len(stream_size(inode, extents)).with_allocated(allocated);
        } else if kind.is_symlink() {
            let target = self.index.target::<D::Error>(inode.id)?;
            metadata = metadata.with_len(target.len() as u64);
        }
        Ok(metadata)
    }
}

impl<D: BlockDevice> FileSystem for ApfsFs<D> {
    type DeviceError = D::Error;

    fn capabilities(&self) -> Capabilities {
        let case = if self.volume.is_case_insensitive() { CaseRule::InsensitivePreserving } else { CaseRule::Sensitive };
        let mut capabilities = Capabilities::new(case, Charset::Unicode, 255)
            .with_symlinks().with_hard_links().with_timestamp_resolution_ns(1);
        for field in [Field::Created, Field::Modified, Field::Changed, Field::Accessed, Field::Permissions, Field::Owner] {
            capabilities = capabilities.with_stored(field, Stored::Yes);
        }
        capabilities
    }

    fn root(&self) -> NodeId {
        NodeId::new(crate::types::filesystem::INODE_ROOT_DIRECTORY).unwrap()
    }

    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        let summary = self.container.space_manager_summary(&self.checkpoint).await?;
        if summary.main_device.free_count > summary.main_device.block_count {
            return Err(Error::new(ErrorKind::Corrupt, "APFS free count exceeds container"));
        }
        if summary.tier2_device.is_some() {
            return Err(Error::new(ErrorKind::Unsupported, "APFS Fusion containers"));
        }
        let files = self.volume.number_files.checked_add(self.volume.number_directories)
            .and_then(|count| count.checked_add(self.volume.number_symlinks)).ok_or(ErrorKind::Corrupt)?;
        Ok(FsStats::new(summary.main_device.block_count, summary.main_device.free_count, self.checkpoint.block_size)
            .with_file_count(files))
    }

    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let name = self.volume.name()?;
        if name.is_empty() { return Ok(None); }
        let target = buf.get_mut(..name.len()).ok_or(ErrorKind::LimitExceeded)?;
        target.copy_from_slice(name.as_bytes());
        Ok(Some(core::str::from_utf8(target).map_err(|_| ErrorKind::Corrupt)?))
    }

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        self.directory(dir).await?;
        let query = name.to_str().map_err(|_| ErrorKind::NotFound)?;
        let entries = self.index.directories.get(&dir.get()).ok_or(ErrorKind::NotFound)?;
        let entry = entries.iter().find(|entry| entry.name == query)
            .or_else(|| entries.iter().find(|entry| filesystem::names_match(&entry.name, query, self.volume.is_case_insensitive())))
            .ok_or(ErrorKind::NotFound)?;
        let node = NodeId::new(entry.file_id).ok_or(ErrorKind::Corrupt)?;
        self.inode(node).await?;
        Ok(node)
    }

    fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let inode = self.directory(dir).await?;
        if dir == self.root() { return Ok(dir); }
        let parent = NodeId::new(inode.parent_id).ok_or(ErrorKind::Corrupt)?;
        self.directory(parent).await?;
        Ok(parent)
    }

    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        let inode = self.inode(node).await?;
        self.metadata(&inode).await
    }

    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        self.directory(dir).await?;
        if from.into_raw() > DirCursor::MAX_RAW { return Err(ErrorKind::InvalidInput.into()); }
        let Some(entries) = self.index.directories.get(&dir.get()) else { return Ok(None); };
        let index = usize::try_from(from.into_raw()).map_err(|_| ErrorKind::InvalidInput)?;
        let Some(entry) = entries.get(index) else {
            return Ok(None);
        };
        let next = from.into_raw().checked_add(1).filter(|value| *value <= DirCursor::MAX_RAW)
            .ok_or(ErrorKind::LimitExceeded)?;
        let node = NodeId::new(entry.file_id).ok_or(ErrorKind::Corrupt)?;
        let name = entry.name.clone();
        let inode = self.inode(node).await?;
        let metadata = self.metadata(&inode).await?;
        Ok(Some(DirEntry::new(Name::from_bytes(name.as_bytes()), node, metadata, DirCursor::from_raw(next))?))
    }

    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let inode = self.inode(node).await?;
        if !file_type::<D::Error>(&inode)?.is_symlink() { return Err(ErrorKind::InvalidInput.into()); }
        let target = self.index.target::<D::Error>(node.get())?;
        let output = buf.get_mut(..target.len()).ok_or(ErrorKind::LimitExceeded)?;
        output.copy_from_slice(target);
        Ok(output)
    }

    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        let inode = self.inode(node).await?;
        match file_type::<D::Error>(&inode)? {
            FileType::Dir => return Err(ErrorKind::IsADirectory.into()),
            FileType::Symlink => return Err(ErrorKind::Symlink.into()),
            FileType::File => {},
            _ => return Err(ErrorKind::Unsupported.into()),
        }
        if mode == OpenMode::Write { return Err(ErrorKind::ReadOnly.into()); }
        if inode.is_compressed() { return Err(crate::ApfsError::Unsupported("compressed file data").into()); }
        Ok(())
    }

    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let inode = self.inode(node).await?;
        match file_type::<D::Error>(&inode)? {
            FileType::Dir => return Err(ErrorKind::IsADirectory.into()),
            FileType::Symlink => return Err(ErrorKind::Symlink.into()),
            FileType::File => {},
            _ => return Err(ErrorKind::Unsupported.into()),
        }
        if inode.is_compressed() { return Err(crate::ApfsError::Unsupported("compressed file data").into()); }
        let extents = self.index.extents(inode.private_id);
        self.container.read_extents_at(extents, stream_size(&inode, extents), offset, buf).await
    }
}
}

use super::Container;
use super::FileSystem;
use super::storage::BlockDevice;
use crate::types::filesystem::{
    self, DirectoryEntryRecord, FileExtentRecord, FileSystemKey, XattrRecord, stream_size,
};
use crate::types::{ContainerSuperblock, InodeRecord, VolumeSuperblock};
use crate::{Detail, VolumeSelector};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use hadris_fs::{
    Capabilities, CaseRule, Charset, DateTime, DirCursor, DirEntry, Error, ErrorKind, Field,
    FileType, FsResult, FsStats, Metadata, MountError, MountOptions, Name, NameError, NodeId,
    OpenMode, Owner, Permissions, Stored,
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

/// The directory entry type an inode's mode implies (`DT_*`).
const fn entry_type(kind: FileType) -> u16 {
    match kind {
        FileType::File => 8,
        FileType::Dir => 4,
        FileType::Symlink => 10,
        FileType::CharDevice => 2,
        FileType::BlockDevice => 6,
        FileType::Fifo => 1,
        FileType::Socket => 12,
        _ => 0,
    }
}

/// The metadata listed for an entry whose node cannot be described: the
/// type its directory record holds and nothing else.
fn damaged(flags: u16) -> Metadata {
    let kind = match flags & filesystem::DREC_TYPE_MASK {
        4 => FileType::Dir,
        10 => FileType::Symlink,
        2 => FileType::CharDevice,
        6 => FileType::BlockDevice,
        1 => FileType::Fifo,
        12 => FileType::Socket,
        _ => FileType::File,
    };
    Metadata::new(kind, Permissions::new(0))
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

/// One indexed directory record; `name` holds the on-disk bytes.
#[derive(Debug)]
struct Entry {
    file_id: u64,
    flags: u16,
    name: Vec<u8>,
}

/// Metadata indexed at mount. Records that cannot be used damage only the
/// nodes they describe: such a node stays listed and fails `stat`, `read`
/// and `readlink` with the reason in `damaged`.
#[derive(Debug, Default)]
struct Index {
    inodes: BTreeMap<u64, InodeRecord>,
    damaged: BTreeMap<u64, &'static str>,
    directories: BTreeMap<u64, Vec<Entry>>,
    extents: BTreeMap<u64, Vec<FileExtentRecord>>,
    symlinks: BTreeMap<u64, (u16, Vec<u8>)>,
}

impl Index {
    fn parse<E>(
        entries: Vec<crate::types::OwnedEntry>,
        sealed_extents: Vec<FileExtentRecord>,
        layout: filesystem::DirectoryKeyLayout,
        limit: Option<usize>,
    ) -> FsResult<Self, E> {
        let mut index = Self::default();
        let mut damaged_streams = BTreeSet::new();
        let mut extents = sealed_extents;
        for entry in entries {
            let key = FileSystemKey::parse(&entry.key)?;
            match key.record_type {
                filesystem::FS_TYPE_INODE => {
                    match InodeRecord::parse(&entry.key, &entry.value) {
                        Ok(inode) => {
                            if index.inodes.insert(inode.id, inode).is_some() {
                                index.damaged.insert(inode.id, "duplicate APFS inode");
                            }
                        }
                        Err(_) => {
                            index.damaged.insert(key.id, "malformed APFS inode record");
                        }
                    }
                    if limit.is_some_and(|limit| index.inodes.len() > limit) {
                        return Err(ErrorKind::LimitExceeded.into());
                    }
                }
                filesystem::FS_TYPE_DIRECTORY_RECORD => {
                    let Ok(record) = DirectoryEntryRecord::parse(&entry.key, &entry.value, layout)
                    else {
                        continue;
                    };
                    if Name::new(record.name).check().is_err() {
                        continue;
                    }
                    index
                        .directories
                        .entry(record.parent_id)
                        .or_default()
                        .push(Entry {
                            file_id: record.file_id,
                            flags: record.flags,
                            name: record.name.to_vec(),
                        });
                }
                filesystem::FS_TYPE_FILE_EXTENT => {
                    match FileExtentRecord::parse(&entry.key, &entry.value) {
                        Ok(extent) => extents.push(extent),
                        Err(_) => {
                            damaged_streams.insert(key.id);
                        }
                    }
                }
                filesystem::FS_TYPE_XATTR => {
                    let Ok(xattr) = XattrRecord::parse(&entry.key, &entry.value) else {
                        continue;
                    };
                    if xattr.name == filesystem::SYMLINK_XATTR_NAME
                        && index
                            .symlinks
                            .insert(xattr.id, (xattr.flags, xattr.data.to_vec()))
                            .is_some()
                    {
                        index
                            .damaged
                            .insert(xattr.id, "duplicate APFS symlink target");
                    }
                }
                _ => {}
            }
        }
        for extent in extents {
            if extent.logical_address.checked_add(extent.length).is_none() {
                damaged_streams.insert(extent.id);
            } else {
                index.extents.entry(extent.id).or_default().push(extent);
            }
        }
        for (&id, extents) in &mut index.extents {
            extents.sort_by_key(|extent| extent.logical_address);
            let mut end = 0;
            for extent in extents {
                if extent.logical_address < end {
                    damaged_streams.insert(id);
                }
                end = extent.logical_address + extent.length;
            }
        }
        index.damage_streams(&damaged_streams, "damaged APFS file extents");
        Ok(index)
    }

    fn damage_streams(&mut self, streams: &BTreeSet<u64>, reason: &'static str) {
        if streams.is_empty() {
            return;
        }
        for inode in self.inodes.values() {
            if streams.contains(&inode.private_id) {
                self.damaged.entry(inode.id).or_insert(reason);
            }
        }
    }

    fn validate(&mut self, block_size: u32, block_count: u64) {
        let mut damaged = Vec::new();
        for (&parent, entries) in &self.directories {
            for entry in entries {
                let Some(inode) = self.inodes.get(&entry.file_id) else {
                    damaged.push((entry.file_id, "APFS directory entry inode missing"));
                    continue;
                };
                let Ok(kind) = file_type::<()>(inode) else {
                    continue;
                };
                let recorded = entry.flags & filesystem::DREC_TYPE_MASK;
                if recorded != 0 && recorded != entry_type(kind) {
                    damaged.push((
                        entry.file_id,
                        "APFS directory entry type disagrees with inode",
                    ));
                } else if kind.is_dir() && (inode.parent_id != parent || inode.id == parent) {
                    damaged.push((entry.file_id, "APFS directory parent disagrees with inode"));
                }
            }
        }
        for (id, reason) in damaged {
            self.damaged.entry(id).or_insert(reason);
        }
        let outside: BTreeSet<u64> = self
            .extents
            .iter()
            .filter(|(_, extents)| {
                extents.iter().any(|extent| {
                    extent.physical_block != 0
                        && extent
                            .physical_block
                            .checked_add(extent.length.div_ceil(u64::from(block_size)))
                            .is_none_or(|end| end > block_count)
                })
            })
            .map(|(&id, _)| id)
            .collect();
        self.damage_streams(&outside, "APFS file extent outside container");
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
///
/// A record that cannot be used damages only its node: the entry is listed
/// with the type its directory record holds, and `stat`, `open`, `read` and
/// `readlink` on the node return the error. Names are exposed as their
/// on-disk bytes, without UTF-8 validation or a 255-byte limit.
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
    /// require [`Self::mount_volume`] and fail with `InvalidInput`. Container
    /// or selected-volume incompatible features this reader cannot read fail
    /// with `Unsupported` and [`Detail::Feature`]; see
    /// [`ContainerSuperblock::check_incompatible_features`] and
    /// [`VolumeSuperblock::check_incompatible_features`].
    /// Mount failures return the device for inspection or retry.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn mount(device: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, None, None).await
    }

    /// Mounts exactly one volume selected by index, object identifier, UUID or
    /// exact name. Missing selectors fail with `NotFound`; duplicate matches
    /// fail with `InvalidInput`. Names are case sensitive for selection.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn mount_volume(
        device: D,
        options: MountOptions,
        selector: VolumeSelector<'_>,
    ) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, Some(selector), None).await
    }

    /// Mounts the sole software-encrypted, single-key volume using a password.
    ///
    /// `crypto_user` selects a crypto-user UUID, or `None` searches supported
    /// password records. Wrong credentials return `InvalidInput` with
    /// [`Detail::Credentials`], preserving
    /// the device. Passwords are borrowed only during mounting. See
    /// [`Container::unlock_volume`] for supported encryption and resource limits.
    #[cfg(feature = "encryption")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn mount_with_password(
        device: D,
        options: MountOptions,
        password: &[u8],
        crypto_user: Option<[u8; 16]>,
    ) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, None, Some((password, crypto_user))).await
    }

    /// Mounts an explicitly selected software-encrypted volume using a password.
    ///
    /// Volume selection is independent of the optional crypto-user UUID.
    /// Credentials and limits behave as in [`Self::mount_with_password`].
    #[cfg(feature = "encryption")]
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn mount_volume_with_password(
        device: D,
        options: MountOptions,
        selector: VolumeSelector<'_>,
        password: &[u8],
        crypto_user: Option<[u8; 16]>,
    ) -> Result<Self, MountError<D, D::Error>> {
        Self::mount_selected(device, options, Some(selector), Some((password, crypto_user))).await
    }

    async fn mount_selected(
        device: D,
        options: MountOptions,
        selector: Option<VolumeSelector<'_>>,
        credentials: Option<(&[u8], Option<[u8; 16]>)>,
    ) -> Result<Self, MountError<D, D::Error>> {
        let mut container = Container::try_open(device).await?;
        match Self::select_volume(&mut container, selector, options.node_limit(), credentials).await {
            Ok((checkpoint, volume, index)) => Ok(Self { container, checkpoint, volume, index }),
            Err(error) => Err(MountError::new(error, container.into_inner())),
        }
    }

    async fn select_volume(
        container: &mut Container<D>,
        selector: Option<VolumeSelector<'_>>,
        limit: Option<usize>,
        credentials: Option<(&[u8], Option<[u8; 16]>)>,
    ) -> FsResult<(ContainerSuperblock, VolumeSuperblock, Index), D::Error> {
        let checkpoint = container.latest_superblock().await?;
        checkpoint.check_incompatible_features()?;
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
        volume.check_incompatible_features()?;
        if checkpoint.flags & 4 != 0 && volume.flags & crate::types::volume::APFS_FS_UNENCRYPTED == 0 {
            if volume.flags & crate::types::volume::APFS_FS_ONEKEY == 0 {
                return Err(crate::ApfsError::Unsupported("APFS per-file-key encryption").into());
            }
            if credentials.is_none() {
                return Err(crate::ApfsError::Unsupported("encrypted APFS volume requires credentials").into());
            }
        }
        if let Some((password, crypto_user)) = credentials {
            #[cfg(feature = "encryption")]
            container.unlock_selected(&checkpoint, &volume, password, crypto_user).await?;
            #[cfg(not(feature = "encryption"))]
            {
                let _ = (password, crypto_user);
                return Err(ErrorKind::Unsupported.into());
            }
        }
        let entries = container.filesystem_root_owned_entries(&volume).await?;
        let sealed_extents = container.sealed_file_extents(&volume).await?;
        let mut index = Index::parse(entries, sealed_extents, volume.directory_key_layout(), limit)?;
        index.validate(checkpoint.block_size, checkpoint.block_count);
        if let Some(reason) = index.damaged.get(&crate::types::filesystem::INODE_ROOT_DIRECTORY) {
            return Err(Error::new(ErrorKind::Corrupt, reason));
        }
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
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.container.into_inner())
    }

    async fn inode(&mut self, node: NodeId) -> FsResult<InodeRecord, D::Error> {
        if let Some(reason) = self.index.damaged.get(&node.get()) {
            return Err(Error::new(ErrorKind::Corrupt, reason));
        }
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

impl<D: BlockDevice> ApfsFs<D> {

    /// Performs the [`FileSystem::capabilities`] operation.
    pub fn capabilities(&self) -> Capabilities {
        let case = if self.volume.is_case_insensitive() { CaseRule::InsensitivePreserving } else { CaseRule::Sensitive };
        let max_name = match self.volume.directory_key_layout() {
            filesystem::DirectoryKeyLayout::Hashed => 0x3ff - 1,
            filesystem::DirectoryKeyLayout::Plain => u16::MAX as usize - 1,
        };
        let mut capabilities = Capabilities::new(case, Charset::Unicode, max_name)
            .with_symlinks().with_hard_links().with_timestamp_resolution_ns(1);
        for field in [Field::Created, Field::Modified, Field::Changed, Field::Accessed, Field::Permissions, Field::Owner] {
            capabilities = capabilities.with_stored(field, Stored::Yes);
        }
        capabilities
    }

    /// Performs the [`FileSystem::root`] operation.
    pub fn root(&self) -> NodeId {
        NodeId::new(crate::types::filesystem::INODE_ROOT_DIRECTORY).unwrap()
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all))]
    /// Performs the [`FileSystem::statfs`] operation.
    pub async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
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

    /// Performs the [`FileSystem::label`] operation.
    pub async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let name = self.volume.name()?;
        if name.is_empty() { return Ok(None); }
        let target = buf.get_mut(..name.len()).ok_or(ErrorKind::LimitExceeded)?;
        target.copy_from_slice(name.as_bytes());
        Ok(Some(core::str::from_utf8(target).map_err(|_| ErrorKind::Corrupt)?))
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(dir = ?dir)))]
    /// Performs the [`FileSystem::lookup`] operation.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        self.directory(dir).await?;
        let entries = self.index.directories.get(&dir.get()).ok_or(ErrorKind::NotFound)?;
        let comparison = self.volume.name_comparison();
        let entry = entries
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .or_else(|| {
                let wanted = name.to_str().ok()?;
                entries.iter().find(|entry| {
                    core::str::from_utf8(&entry.name)
                        .is_ok_and(|stored| comparison.matches(stored, wanted))
                })
            })
            .ok_or(ErrorKind::NotFound)?;
        NodeId::new(entry.file_id).ok_or_else(|| ErrorKind::Corrupt.into())
    }

    /// Performs the [`FileSystem::forget`] operation.
    pub fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(dir = ?dir)))]
    /// Performs the [`FileSystem::parent`] operation.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let inode = self.directory(dir).await?;
        if dir == self.root() { return Ok(dir); }
        let parent = NodeId::new(inode.parent_id).ok_or(ErrorKind::Corrupt)?;
        self.directory(parent).await?;
        Ok(parent)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(node = ?node)))]
    /// Performs the [`FileSystem::stat`] operation.
    pub async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        let inode = self.inode(node).await?;
        self.metadata(&inode).await
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(dir = ?dir)))]
    /// Performs the [`FileSystem::readdir`] operation.
    pub async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
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
        let (name, flags) = (entry.name.clone(), entry.flags);
        let metadata = match self.inode(node).await {
            Ok(inode) => self.metadata(&inode).await,
            Err(error) => Err(error),
        };
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::Io => return Err(error),
            Err(_) => damaged(flags),
        };
        DirEntry::new(Name::new(&name), node, metadata, DirCursor::from_raw(next))
            .map(Some)
            .map_err(|error| match error {
                NameError::TooLong => ErrorKind::NameTooLong.into(),
                error => error.into(),
            })
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(node = ?node, bytes = buf.len())))]
    /// Performs the [`FileSystem::readlink`] operation.
    pub async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let inode = self.inode(node).await?;
        if !file_type::<D::Error>(&inode)?.is_symlink() { return Err(ErrorKind::InvalidInput.into()); }
        let target = self.index.target::<D::Error>(node.get())?;
        let output = buf.get_mut(..target.len()).ok_or(ErrorKind::LimitExceeded)?;
        output.copy_from_slice(target);
        Ok(output)
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(node = ?node)))]
    /// Performs the [`FileSystem::open`] operation.
    pub async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
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

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(node = ?node)))]
    /// Performs the [`FileSystem::close`] operation.
    pub async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::apfs", level = "trace", skip_all, fields(node = ?node, offset = offset, bytes = buf.len())))]
    /// Performs the [`FileSystem::read`] operation.
    pub async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        #[cfg(feature = "encryption")]
        self.container.check_unlocked_volume(&self.volume)?;
        let inode = self.inode(node).await?;
        match file_type::<D::Error>(&inode)? {
            FileType::Dir => return Err(ErrorKind::IsADirectory.into()),
            FileType::Symlink => return Err(ErrorKind::Symlink.into()),
            FileType::File => {},
            _ => return Err(ErrorKind::Unsupported.into()),
        }
        if inode.is_compressed() { return Err(crate::ApfsError::Unsupported("compressed file data").into()); }
        let extents = self.index.extents(inode.private_id);
        self.container.read_volume_extents_at(&self.volume, extents, stream_size(&inode, extents), offset, buf).await
    }
}
hadris_fs::__forward_filesystem! {
    async fn ApfsFs<D: BlockDevice> as FileSystem { read }
}

send_filesystem! {
hadris_fs::__forward_filesystem! {
    async fn ApfsFs<D: hadris_storage::async_::SendBlockDevice> as hadris_fs::async_::FileSystem { read }
}
}

}

//! Mode-independent state of an open volume and the decoding of its
//! structures.

use hadris_fs::{
    Capabilities, CaseRule, Charset, Field, FileType, Metadata, NodeId, Owner, Permissions, Stored,
};

use crate::UdfRevision;
use crate::error::{Detail, Error};
use crate::raw::{self, ExtendedFileEntry, FileEntry, IcbFlags, Tag, file_type, tag};
use crate::time::to_datetime;

/// The largest logical or device block the reader buffers.
pub(crate) const MAX_BLOCK: usize = 4096;
/// The most partition maps a logical volume may have.
pub(crate) const MAX_PARTITIONS: usize = 8;

/// A partition of the logical volume, in the order of its partition maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Partition {
    number: u16,
    start: u32,
    len: u32,
}

impl Partition {
    pub(crate) const fn new(number: u16, start: u32, len: u32) -> Self {
        Self { number, start, len }
    }

    /// The partition number of its Partition Descriptor.
    pub const fn number(&self) -> u16 {
        self.number
    }

    /// The first logical block of the partition on the device.
    pub const fn start(&self) -> u32 {
        self.start
    }

    /// The length in logical blocks.
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Whether the partition has no blocks.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// An ICB location: partition reference and logical block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Location {
    pub(crate) partition: u16,
    pub(crate) block: u32,
}

impl Location {
    pub(crate) fn id(self) -> NodeId {
        NodeId::new(((u64::from(self.partition) << 32) | u64::from(self.block)) + 1)
            .expect("a 48-bit location plus one is never zero")
    }

    pub(crate) fn of(node: NodeId) -> Option<Self> {
        let raw = node.get().checked_sub(1)?;
        let partition = u16::try_from(raw >> 32).ok()?;
        Some(Self {
            partition,
            block: raw as u32,
        })
    }

    pub(crate) fn from_long(ad: &raw::LongAd) -> Self {
        Self {
            partition: ad.location.partition.get(),
            block: ad.location.block.get(),
        }
    }
}

/// A decoded d-string of an `N / 2`-byte field: at most `N / 2 - 2`
/// characters of one or two UTF-8 bytes.
#[derive(Clone, Copy)]
pub(crate) struct Identifier<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Identifier<N> {
    pub(crate) fn decode(field: &[u8]) -> Self {
        let mut bytes = [0u8; N];
        let len = crate::name::decode_dstring(field, &mut bytes);
        Self { bytes, len }
    }

    pub(crate) fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> core::fmt::Debug for Identifier<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

/// What a mounted volume needs to find anything, read once at open.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Info {
    pub(crate) block_size: u32,
    pub(crate) len: u64,
    pub(crate) partitions: [Partition; MAX_PARTITIONS],
    pub(crate) partition_count: usize,
    pub(crate) root: Location,
    pub(crate) revision: UdfRevision,
    pub(crate) volume_id: Identifier<64>,
    pub(crate) logical_volume_id: Identifier<256>,
    pub(crate) free_blocks: Option<u64>,
}

impl Info {
    pub(crate) fn partitions(&self) -> &[Partition] {
        &self.partitions[..self.partition_count]
    }

    /// The byte offset of `count` bytes from logical block `block` of
    /// partition `partition`, checked against the partition.
    pub(crate) fn offset<E>(
        &self,
        partition: u16,
        block: u32,
        count: u64,
    ) -> Result<u64, Error<E>> {
        let part = self
            .partitions()
            .get(usize::from(partition))
            .ok_or(Detail::Partition.corrupt())?;
        let bs = u64::from(self.block_size);
        let blocks = count.div_ceil(bs);
        if u64::from(block) + blocks > u64::from(part.len) {
            return Err(Detail::Partition.corrupt());
        }
        Ok((u64::from(part.start) + u64::from(block)) * bs)
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        Capabilities::new(CaseRule::Sensitive, Charset::Unicode, 508)
            .with_symlinks()
            .with_hard_links()
            .with_stored(Field::Created, Stored::Partial)
            .with_stored(Field::Modified, Stored::Yes)
            .with_stored(Field::Accessed, Stored::Yes)
            .with_stored(Field::Changed, Stored::Yes)
            .with_stored(Field::Permissions, Stored::Yes)
            .with_stored(Field::Owner, Stored::Yes)
            .with_timestamp_resolution_ns(1000)
    }
}

/// A File Entry or Extended File Entry, decoded, with the block that holds
/// it for its allocation descriptors.
pub(crate) struct Icb {
    pub(crate) at: Location,
    pub(crate) file_type: u8,
    pub(crate) flags: IcbFlags,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) permissions: u32,
    pub(crate) link_count: u16,
    pub(crate) size: u64,
    pub(crate) times: Times,
    pub(crate) ad_start: usize,
    pub(crate) ad_len: usize,
    pub(crate) block: [u8; MAX_BLOCK],
}

impl Icb {
    /// Decodes the entry in `block`, the logical block at `at`.
    pub(crate) fn parse<E>(
        at: Location,
        block: [u8; MAX_BLOCK],
        block_size: usize,
    ) -> Result<Self, Error<E>> {
        let data = &block[..block_size];
        let tag = check_tag(data, None, at.block).map_err(|_| Detail::Icb.corrupt())?;
        let bad = || Detail::Icb.corrupt();
        let (fixed, header): (usize, Header) = match tag.identifier.get() {
            tag::FILE_ENTRY => {
                let fe: FileEntry = bytemuck::pod_read_unaligned(data.get(..176).ok_or_else(bad)?);
                let times = Times {
                    created: None,
                    modified: to_datetime(&fe.modified),
                    accessed: to_datetime(&fe.accessed),
                    changed: to_datetime(&fe.attributes_changed),
                };
                (
                    176,
                    Header {
                        icb_tag: fe.icb_tag,
                        uid: fe.uid.get(),
                        gid: fe.gid.get(),
                        permissions: fe.permissions.get(),
                        link_count: fe.link_count.get(),
                        size: fe.information_length.get(),
                        times,
                        ea: fe.extended_attributes_length.get(),
                        ad: fe.allocation_descriptors_length.get(),
                    },
                )
            }
            tag::EXTENDED_FILE_ENTRY => {
                let efe: ExtendedFileEntry =
                    bytemuck::pod_read_unaligned(data.get(..216).ok_or_else(bad)?);
                let times = Times {
                    created: to_datetime(&efe.created),
                    modified: to_datetime(&efe.modified),
                    accessed: to_datetime(&efe.accessed),
                    changed: to_datetime(&efe.attributes_changed),
                };
                (
                    216,
                    Header {
                        icb_tag: efe.icb_tag,
                        uid: efe.uid.get(),
                        gid: efe.gid.get(),
                        permissions: efe.permissions.get(),
                        link_count: efe.link_count.get(),
                        size: efe.information_length.get(),
                        times,
                        ea: efe.extended_attributes_length.get(),
                        ad: efe.allocation_descriptors_length.get(),
                    },
                )
            }
            _ => return Err(bad()),
        };
        let ad_start = usize::try_from(header.ea)
            .ok()
            .and_then(|ea| fixed.checked_add(ea))
            .ok_or_else(bad)?;
        let ad_len = usize::try_from(header.ad).map_err(|_| bad())?;
        if ad_start
            .checked_add(ad_len)
            .is_none_or(|end| end > block_size)
        {
            return Err(bad());
        }
        Ok(Self {
            at,
            file_type: header.icb_tag.file_type,
            flags: IcbFlags::from_bits_retain(header.icb_tag.flags.get()),
            uid: header.uid,
            gid: header.gid,
            permissions: header.permissions,
            link_count: header.link_count,
            size: header.size,
            times: header.times,
            ad_start,
            ad_len,
            block,
        })
    }

    pub(crate) fn allocation(&self) -> u16 {
        (self.flags & IcbFlags::ALLOCATION).bits()
    }

    pub(crate) fn fs_type(&self) -> Option<FileType> {
        Some(match self.file_type {
            file_type::DIRECTORY | file_type::STREAM_DIRECTORY => FileType::Dir,
            file_type::FILE | file_type::UNSPECIFIED | file_type::REAL_TIME_FILE => FileType::File,
            file_type::SYMLINK => FileType::Symlink,
            file_type::BLOCK_DEVICE => FileType::BlockDevice,
            file_type::CHAR_DEVICE => FileType::CharDevice,
            file_type::FIFO => FileType::Fifo,
            file_type::SOCKET => FileType::Socket,
            _ => return None,
        })
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.fs_type() == Some(FileType::Dir)
    }

    pub(crate) fn metadata(&self, file_type: FileType) -> Metadata {
        let nlink = u64::from(self.link_count.max(1)) + u64::from(file_type.is_dir());
        let mut meta = Metadata::new(
            file_type,
            Permissions::new(mode_of(self.permissions, self.flags)),
        )
        .with_len(self.size)
        .with_nlink(nlink);
        if self.uid != u32::MAX || self.gid != u32::MAX {
            meta = meta.with_owner(Owner::new(self.uid, self.gid));
        }
        if let Some(time) = self.times.created {
            meta = meta.with_created(time);
        }
        if let Some(time) = self.times.modified {
            meta = meta.with_modified(time);
        }
        if let Some(time) = self.times.accessed {
            meta = meta.with_accessed(time);
        }
        if let Some(time) = self.times.changed {
            meta = meta.with_changed(time);
        }
        meta
    }
}

/// The times of a file entry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Times {
    created: Option<hadris_fs::DateTime>,
    modified: Option<hadris_fs::DateTime>,
    accessed: Option<hadris_fs::DateTime>,
    changed: Option<hadris_fs::DateTime>,
}

struct Header {
    icb_tag: raw::IcbTag,
    uid: u32,
    gid: u32,
    permissions: u32,
    link_count: u16,
    size: u64,
    times: Times,
    ea: u32,
    ad: u32,
}

/// Checks a descriptor tag: checksum, version, reserved byte, CRC, the
/// identifier when `id` is given, and the location.
pub(crate) fn check_tag(data: &[u8], id: Option<u16>, location: u32) -> Result<Tag, ()> {
    let tag = Tag::read(data).ok_or(())?;
    if !tag.is_checksum_valid()
        || !matches!(tag.version.get(), 2 | 3)
        || tag.reserved != 0
        || id.is_some_and(|id| tag.identifier.get() != id)
        || tag.location.get() != location
        || !tag.is_crc_valid(&data[Tag::SIZE..])
    {
        return Err(());
    }
    Ok(tag)
}

/// POSIX permission bits of UDF permissions and ICB flags.
pub(crate) fn mode_of(permissions: u32, flags: IcbFlags) -> u32 {
    let mut mode = (permissions & 0o7) | ((permissions >> 2) & 0o70) | ((permissions >> 4) & 0o700);
    if flags.contains(IcbFlags::SETUID) {
        mode |= 0o4000;
    }
    if flags.contains(IcbFlags::SETGID) {
        mode |= 0o2000;
    }
    if flags.contains(IcbFlags::STICKY) {
        mode |= 0o1000;
    }
    mode
}

/// UDF permissions and ICB flags of POSIX permission bits. The owner may
/// also change attributes and delete, as Linux records it.
#[cfg(feature = "alloc")]
pub(crate) fn permissions_of(mode: u32) -> (u32, IcbFlags) {
    let permissions = (mode & 0o7)
        | ((mode & 0o70) << 2)
        | ((mode & 0o700) << 4)
        | (raw::Permissions::OWNER_CHANGE_ATTRIBUTES | raw::Permissions::OWNER_DELETE).bits();
    let mut flags = IcbFlags::empty();
    if mode & 0o4000 != 0 {
        flags |= IcbFlags::SETUID;
    }
    if mode & 0o2000 != 0 {
        flags |= IcbFlags::SETGID;
    }
    if mode & 0o1000 != 0 {
        flags |= IcbFlags::STICKY;
    }
    (permissions, flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_encode_the_icb_location() {
        let at = Location {
            partition: 3,
            block: 0,
        };
        assert_ne!(at.id().get(), 0);
        assert_eq!(Location::of(at.id()), Some(at));
        assert_eq!(Location::of(NodeId::new(u64::MAX).unwrap()), None);
    }

    #[test]
    fn tags_are_checked_for_version_reserved_byte_location_and_crc() {
        let mut block = [0u8; 64];
        block[16..20].copy_from_slice(b"body");
        Tag::seal(&mut block, tag::PRIMARY_VOLUME, 3, 17, 48);
        assert!(check_tag(&block, Some(tag::PRIMARY_VOLUME), 17).is_ok());
        assert!(check_tag(&block, Some(tag::LOGICAL_VOLUME), 17).is_err());
        assert!(check_tag(&block, None, 18).is_err());

        let mut old = block;
        Tag::seal(&mut old, tag::PRIMARY_VOLUME, 2, 17, 48);
        assert!(check_tag(&old, None, 17).is_ok());
        Tag::seal(&mut old, tag::PRIMARY_VOLUME, 1, 17, 48);
        assert!(check_tag(&old, None, 17).is_err());

        let mut reserved = block;
        reserved[5] = 1;
        reserved[4] = Tag::checksum_of(reserved[..16].try_into().unwrap());
        assert!(check_tag(&reserved, None, 17).is_err());

        let mut body = block;
        body[40] ^= 1;
        assert!(check_tag(&body, None, 17).is_err());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn permissions_round_trip() {
        for mode in [0o755, 0o644, 0o4711, 0o2750, 0o1777, 0] {
            let (perms, flags) = permissions_of(mode);
            assert_eq!(mode_of(perms, flags), mode);
        }
        assert_eq!(mode_of(0x7FFF, IcbFlags::empty()), 0o777);
    }
}

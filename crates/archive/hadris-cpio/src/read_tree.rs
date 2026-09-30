use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use hadris_fs::{
    Content, DeviceNumber, ErrorKind, FileType, Node, Owner, PathError, SetAttr, Tree,
};

use super::io::Read;
use super::read::{CpioReader, Entry};
use crate::error::Error;

/// Bytes read per request while loading an entry's data.
const CHUNK: usize = 64 * 1024;

/// The path of an entry relative to the archive root: leading `/` and
/// `./` removed. `None` for the root itself.
fn relative_path(name: &[u8]) -> Option<&[u8]> {
    let mut path = name;
    loop {
        if let Some(rest) = path.strip_prefix(b"/") {
            path = rest;
        } else if let Some(rest) = path.strip_prefix(b"./") {
            path = rest;
        } else {
            break;
        }
    }
    match path {
        b"" | b"." => None,
        path => Some(path),
    }
}

/// Adds `node` at `path`, replacing what an earlier entry put there, as
/// extraction does.
fn put(tree: &mut Tree, path: &[u8], node: Node) -> Result<(), PathError> {
    match tree.get(path) {
        Some(_) => tree.replace(path, node),
        None => tree.insert(path, node),
    }
}

/// Makes `path` another name of the node at `existing`, replacing what an
/// earlier entry put there as [`put`] does.
fn put_link(tree: &mut Tree, existing: &[u8], path: &[u8]) -> Result<(), PathError> {
    if tree.get(path).is_some() {
        tree.replace(path, Node::file(Content::empty()))?;
        tree.remove(path)?;
    }
    tree.link(existing, path)
}

/// The key of a hard link group: device major and minor, and inode.
type GroupKey = (u32, u32, u64);

/// A hard link group seen so far: the names that still hold its node, and
/// the node, from the last entry that carried data.
struct Group {
    names: Vec<Vec<u8>>,
    node: Node,
}

/// The hard link groups, with the group each name belongs to, so a later
/// entry of that name takes it out of its group.
#[derive(Default)]
struct Groups {
    groups: BTreeMap<GroupKey, Group>,
    owners: BTreeMap<Vec<u8>, GroupKey>,
}

impl Groups {
    /// Takes `path` out of the group that holds it.
    fn release(&mut self, path: &[u8]) {
        if let Some(key) = self.owners.remove(path)
            && let Some(group) = self.groups.get_mut(&key)
        {
            group.names.retain(|name| name != path);
        }
    }

    /// Adds the file entry `node` at `path` to the group `key`. An entry
    /// with data, or the first of its group, gives every name of the group
    /// its node; any other becomes a new name of the group's node.
    fn add(
        &mut self,
        tree: &mut Tree,
        key: GroupKey,
        path: &[u8],
        node: Node,
        has_data: bool,
    ) -> Result<(), PathError> {
        self.release(path);
        let group = match self.groups.entry(key) {
            alloc::collections::btree_map::Entry::Occupied(slot) => {
                let group = slot.into_mut();
                if has_data {
                    group.node = node;
                }
                group
            }
            alloc::collections::btree_map::Entry::Vacant(slot) => slot.insert(Group {
                names: Vec::new(),
                node,
            }),
        };
        match group.names.first() {
            Some(first) if !has_data => put_link(tree, first, path)?,
            _ => {
                put(tree, path, group.node.clone())?;
                for name in &group.names {
                    put_link(tree, path, name).map_err(|err| err.with_path(name))?;
                }
            }
        }
        group.names.push(path.to_vec());
        self.owners.insert(path.to_vec(), key);
        Ok(())
    }
}

/// The length of entry data loaded into memory. [`ErrorKind::LimitExceeded`]
/// past what a `Vec` holds on this target, as an `odc` file of 4 GiB or
/// more on a 32-bit one.
fn data_len<E>(len: u64) -> Result<usize, Error<E>> {
    usize::try_from(len)
        .ok()
        .filter(|&len| isize::try_from(len).is_ok())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::LimitExceeded,
                "entry data does not fit in memory",
            )
        })
}

io_transform! {

async fn read_data<R: Read, B: AsRef<[u8]> + AsMut<[u8]> + super::io::MaybeSend>(entry: &mut Entry<'_, R, B>) -> Result<Vec<u8>, Error<R::Error>> {
    let len = data_len(entry.remaining())?;
    let mut data = Vec::new();
    let mut chunk = alloc::vec![0u8; CHUNK.min(len)];
    while data.len() < len {
        let take = (len - data.len()).min(chunk.len());
        let read = entry.read(&mut chunk[..take]).await?;
        if read == 0 {
            return Err(crate::error::Detail::Truncated.corrupt());
        }
        data.extend_from_slice(&chunk[..read]);
    }
    Ok(data)
}

/// Reads the archive from `reader` up to its trailer, or the end of the
/// stream, into a [`Tree`].
///
/// Names lose a leading `/` or `./`, and the entry `.` sets the root's
/// attributes; a name with a `..` component fails with
/// [`ErrorKind::InvalidInput`]. A later entry replaces an earlier one of
/// the same name, in archive order, hard link names included. File data is
/// loaded into memory. Regular files that share an inode and device number
/// and have a link count above 1 become hard links, with the data and
/// attributes of the last name that carries data, as GNU cpio writes them.
/// Errors carry the entry name.
pub async fn read_tree<R: Read, B: AsRef<[u8]> + AsMut<[u8]> + super::io::MaybeSend>(reader: &mut CpioReader<R, B>) -> Result<Tree, PathError> {
    let mut tree = Tree::new();
    let mut groups = Groups::default();
    loop {
        let Some(mut entry) = reader.next_entry().await? else { break };
        let name = entry.path().to_vec();
        let with_path = |err: PathError| err.with_path(&name);
        let owner = Owner::new(entry.uid(), entry.gid());
        let mut attrs = SetAttr::new().with_permissions(entry.permissions()).with_owner(owner);
        if let Some(time) = entry.modified() {
            attrs = attrs.with_modified(time);
        }
        let file_type = entry.file_type();
        let node = match file_type {
            FileType::File => Node::file(Content::bytes(read_data(&mut entry).await.map_err(|err| with_path(err.into()))?)),
            FileType::Dir => Node::dir(),
            FileType::Symlink => Node::symlink(read_data(&mut entry).await.map_err(|err| with_path(err.into()))?),
            FileType::CharDevice | FileType::BlockDevice => Node::special(file_type, Some(entry.rdev())),
            FileType::Fifo | FileType::Socket => Node::special(file_type, None),
            _ => return Err(with_path(PathError::new(ErrorKind::Unsupported, "unknown cpio file type"))),
        }
        .with_attrs(attrs);
        let Some(path) = relative_path(&name) else {
            if file_type != FileType::Dir {
                return Err(with_path(PathError::new(ErrorKind::InvalidInput, "the archive root is not a directory")));
            }
            tree.replace("", node).map_err(with_path)?;
            continue;
        };
        if file_type != FileType::File || entry.nlink() < 2 {
            groups.release(path);
            put(&mut tree, path, node).map_err(with_path)?;
            continue;
        }
        let dev: DeviceNumber = entry.dev();
        let key = (dev.major(), dev.minor(), entry.ino());
        groups.add(&mut tree, key, path, node, entry.len() > 0).map_err(with_path)?;
    }
    Ok(tree)
}

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_past_memory_is_limit_exceeded() {
        assert_eq!(data_len::<()>(5).ok(), Some(5));
        let err = data_len::<()>(u64::MAX).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
        let four_gib = u64::from(u32::MAX) + 1;
        #[cfg(target_pointer_width = "32")]
        assert_eq!(
            data_len::<()>(four_gib).unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(data_len::<()>(four_gib).ok(), Some(1 << 32));
    }
}

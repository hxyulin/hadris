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

/// A hard link group seen so far: its names and the node that carries the
/// data, once read.
struct Group {
    names: Vec<Vec<u8>>,
    node: Option<Node>,
}

io_transform! {

async fn read_data<R: Read>(entry: &mut Entry<'_, R>) -> Result<Vec<u8>, Error<R::Error>> {
    let mut data = Vec::new();
    let mut chunk = alloc::vec![0u8; CHUNK.min(entry.remaining() as usize)];
    while entry.remaining() > 0 {
        let take = (entry.remaining() as usize).min(chunk.len());
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
/// the same name. File data is loaded into memory. Regular files that
/// share an inode and device number and have a link count above 1 become
/// hard links, with the data and attributes of the name that carries data,
/// as GNU cpio writes them. Errors carry the entry name.
pub async fn read_tree<R: Read>(reader: &mut CpioReader<R>) -> Result<Tree, PathError> {
    let mut tree = Tree::new();
    let mut groups: BTreeMap<(u32, u32, u64), Group> = BTreeMap::new();
    loop {
        let Some(mut entry) = reader.next_entry().await? else { break };
        let name = entry.name().to_vec();
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
            put(&mut tree, path, node).map_err(with_path)?;
            continue;
        }
        let dev: DeviceNumber = entry.dev();
        let group = groups
            .entry((dev.major(), dev.minor(), entry.ino()))
            .or_insert_with(|| Group { names: Vec::new(), node: None });
        if group.node.is_none() || entry.len() > 0 {
            group.node = Some(node);
        }
        group.names.push(path.to_vec());
    }
    for group in groups.into_values() {
        let (Some(node), Some((first, rest))) = (group.node, group.names.split_first()) else { continue };
        put(&mut tree, first, node).map_err(|err| err.with_path(first))?;
        for name in rest {
            if tree.get(name).is_some() {
                tree.remove(name).map_err(|err| err.with_path(name))?;
            }
            tree.link(first, name).map_err(|err| err.with_path(name))?;
        }
    }
    Ok(tree)
}

}

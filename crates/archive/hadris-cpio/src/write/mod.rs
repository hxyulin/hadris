/// In-memory file tree representation for archive construction.
pub mod file_tree;

/// Build a [`FileTree`] by scanning the host filesystem (requires `std`).
#[cfg(feature = "std")]
pub mod from_fs;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::super::Write;
use super::header::{CpioMagic, HEADER_SIZE, RawNewcHeader, TRAILER_NAME};
use crate::error::{Error, Result};
use crate::mode::{self, FileType};
use file_tree::{FileNode, FileTree};

/// Options for writing a CPIO archive.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpioWriteOptions {
    /// If true, use the 070702 (CRC) magic; otherwise use 070701 (newc).
    pub use_crc: bool,
}

impl CpioWriteOptions {
    /// Selects CRC newc (`070702`) instead of plain newc (`070701`).
    pub const fn crc(mut self, enabled: bool) -> Self {
        self.use_crc = enabled;
        self
    }
}

/// Stateless encoder used by the owning writer.
struct ArchiveEncoder {
    options: CpioWriteOptions,
}

/// Owning CPIO archive writer with a recoverable output target.
pub struct CpioArchiveWriter<W> {
    writer: W,
    options: CpioWriteOptions,
}

io_transform! {

impl ArchiveEncoder {
    fn new(options: CpioWriteOptions) -> Self {
        Self { options }
    }

    async fn write<W: Write>(&self, writer: &mut W, tree: &FileTree) -> Result<()> {
        let magic = if self.options.use_crc {
            CpioMagic::NewcCrc
        } else {
            CpioMagic::Newc
        };

        // Flatten the tree into (path, node) pairs
        let mut entries: Vec<(String, &FileNode)> = Vec::new();
        for node in &tree.root {
            flatten_tree(node, String::new(), &mut entries);
        }

        // Inode counter and hard link map (link_target path -> inode)
        let mut next_ino: u32 = 1;
        let mut path_to_ino: BTreeMap<String, u32> = BTreeMap::new();
        // Track hard links: maps link_target -> list of (name, ino)
        let mut hard_links: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();

        // First pass: assign inodes and gather hard links
        let mut assigned_inos: Vec<u32> = Vec::with_capacity(entries.len());
        for (path, node) in &entries {
            match node {
                FileNode::HardLink { link_target, .. } => {
                    let ino = *path_to_ino
                        .get(link_target.as_str())
                        .ok_or(Error::UnresolvedHardLink { ino: 0 })?;
                    assigned_inos.push(ino);
                    hard_links
                        .entry(link_target.clone())
                        .or_default()
                        .push((path.clone(), ino));
                }
                _ => {
                    let ino = next_ino;
                    next_ino += 1;
                    path_to_ino.insert(path.clone(), ino);
                    assigned_inos.push(ino);
                }
            }
        }

        // Second pass: write entries
        for (i, (path, node)) in entries.iter().enumerate() {
            let ino = assigned_inos[i];
            self.write_entry(writer, magic, ino, path, node, &hard_links).await?;
        }

        // Write TRAILER!!!
        self.write_trailer(writer, magic).await?;

        Ok(())
    }

    async fn write_entry<W: Write>(
        &self,
        writer: &mut W,
        magic: CpioMagic,
        ino: u32,
        path: &str,
        node: &FileNode,
        hard_links: &BTreeMap<String, Vec<(String, u32)>>,
    ) -> Result<()> {
        let name_bytes = path.as_bytes();
        let namesize = (name_bytes.len() + 1) as u32; // +1 for NUL

        let (file_mode, uid, gid, mtime, filesize, data, nlink, devmajor, devminor, rdevmajor, rdevminor) =
            match node {
                FileNode::File {
                    contents,
                    permissions,
                    uid,
                    gid,
                    mtime,
                    ..
                } => {
                    let nlink = 1 + hard_links.get(path).map_or(0, |v| v.len() as u32);
                    let size = contents.len() as u32;
                    (
                        mode::make_mode(FileType::Regular, *permissions),
                        *uid,
                        *gid,
                        *mtime,
                        size,
                        contents.as_slice(),
                        nlink,
                        0u32,
                        0u32,
                        0u32,
                        0u32,
                    )
                }
                FileNode::Directory {
                    permissions,
                    uid,
                    gid,
                    mtime,
                    ..
                } => (
                    mode::make_mode(FileType::Directory, *permissions),
                    *uid,
                    *gid,
                    *mtime,
                    0u32,
                    &[] as &[u8],
                    2u32,
                    0u32,
                    0u32,
                    0u32,
                    0u32,
                ),
                FileNode::Symlink {
                    target,
                    permissions,
                    uid,
                    gid,
                    mtime,
                    ..
                } => {
                    let data = target.as_bytes();
                    (
                        mode::make_mode(FileType::Symlink, *permissions),
                        *uid,
                        *gid,
                        *mtime,
                        data.len() as u32,
                        data,
                        1u32,
                        0u32,
                        0u32,
                        0u32,
                        0u32,
                    )
                }
                FileNode::HardLink { .. } => {
                    // Hard link: written as a regular file with filesize=0
                    (
                        mode::make_mode(FileType::Regular, 0o644),
                        0u32,
                        0u32,
                        0u32,
                        0u32,
                        &[] as &[u8],
                        2u32,
                        0u32,
                        0u32,
                        0u32,
                        0u32,
                    )
                }
                FileNode::DeviceNode {
                    device_type,
                    major,
                    minor,
                    permissions,
                    uid,
                    gid,
                    mtime,
                    ..
                } => (
                    mode::make_mode(*device_type, *permissions),
                    *uid,
                    *gid,
                    *mtime,
                    0u32,
                    &[] as &[u8],
                    1u32,
                    0u32,
                    0u32,
                    *major,
                    *minor,
                ),
                FileNode::Fifo {
                    permissions,
                    uid,
                    gid,
                    mtime,
                    ..
                } => (
                    mode::make_mode(FileType::Fifo, *permissions),
                    *uid,
                    *gid,
                    *mtime,
                    0u32,
                    &[] as &[u8],
                    1u32,
                    0u32,
                    0u32,
                    0u32,
                    0u32,
                ),
            };

        let check = if magic == CpioMagic::NewcCrc && filesize > 0 {
            compute_crc(data)
        } else {
            0
        };

        let raw = RawNewcHeader::build(
            magic, ino, file_mode, uid, gid, nlink, mtime, filesize, devmajor, devminor,
            rdevmajor, rdevminor, namesize, check,
        );

        raw.write(writer).await?;

        // Write filename + NUL
        writer.write_all(name_bytes).await?;
        writer.write_all(&[0]).await?;

        // Pad to 4-byte boundary after header + namesize
        let header_plus_name = HEADER_SIZE as u64 + namesize as u64;
        let pad = align4_padding(header_plus_name);
        if pad > 0 {
            writer.write_all(&[0u8; 3][..pad as usize]).await?;
        }

        // Write file data
        if filesize > 0 {
            writer.write_all(data).await?;

            // Pad data to 4-byte boundary
            let data_pad = align4_padding(filesize as u64);
            if data_pad > 0 {
                writer.write_all(&[0u8; 3][..data_pad as usize]).await?;
            }
        }

        Ok(())
    }

    async fn write_trailer<W: Write>(&self, writer: &mut W, magic: CpioMagic) -> Result<()> {
        let namesize = (TRAILER_NAME.len() + 1) as u32;
        let raw = RawNewcHeader::build(
            magic, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, namesize, 0,
        );
        raw.write(writer).await?;

        writer.write_all(TRAILER_NAME).await?;
        writer.write_all(&[0]).await?;

        // Pad name
        let header_plus_name = HEADER_SIZE as u64 + namesize as u64;
        let pad = align4_padding(header_plus_name);
        if pad > 0 {
            writer.write_all(&[0u8; 3][..pad as usize]).await?;
        }

        Ok(())
    }
}

impl<W: Write> CpioArchiveWriter<W> {
    /// Creates an owning archive writer.
    pub fn new(writer: W, options: CpioWriteOptions) -> Self {
        Self { writer, options }
    }

    /// Returns the target without writing an archive.
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Writes the complete archive and returns its target.
    pub async fn finish(mut self, tree: &FileTree) -> Result<W> {
        ArchiveEncoder::new(self.options)
            .write(&mut self.writer, tree)
            .await?;
        Ok(self.writer)
    }

    /// Creates a complete archive and returns its target.
    pub async fn create(writer: W, tree: &FileTree, options: CpioWriteOptions) -> Result<W> {
        Self::new(writer, options).finish(tree).await
    }
}

} // io_transform!

/// Flatten a file tree depth-first into a list of (path, node) pairs.
fn flatten_tree<'a>(node: &'a FileNode, prefix: String, out: &mut Vec<(String, &'a FileNode)>) {
    let name = node.node_name();
    let path = if prefix.is_empty() {
        String::from(name)
    } else {
        let mut p = prefix;
        p.push('/');
        p.push_str(name);
        p
    };

    match node {
        FileNode::Directory { children, .. } => {
            let dir_path = path.clone();
            out.push((dir_path.clone(), node));
            for child in children {
                flatten_tree(child, dir_path.clone(), out);
            }
        }
        _ => {
            out.push((path, node));
        }
    }
}

/// Compute the "CRC" checksum for CPIO (sum of all bytes, not a real CRC-32).
fn compute_crc(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    for &b in data {
        sum = sum.wrapping_add(b as u32);
    }
    sum
}

/// Compute the number of padding bytes needed to align `offset` to a 4-byte boundary.
fn align4_padding(offset: u64) -> u64 {
    (4 - (offset % 4)) % 4
}

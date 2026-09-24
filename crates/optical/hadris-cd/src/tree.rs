use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadris_fs::ErrorKind;
use hadris_fs::tree::{Content, NodeKind, Tree};

use hadris_fs::PathError;

use crate::error::Detail;

/// The files of `tree` as the extents the ISO 9660 writer stored them in,
/// with the same directories, symlinks, device nodes, hard links and
/// metadata, for the UDF writer. Also returns the block after the last
/// file.
pub(crate) fn stored(tree: &Tree, iso: &hadris_iso::Report) -> Result<(Tree, u64), PathError> {
    let mut out = Tree::new();
    out.set_metadata("/", *tree.root().metadata())?;
    let mut end = 0u64;
    let mut first: BTreeMap<usize, String> = BTreeMap::new();
    let mut pending: Vec<(hadris_fs::tree::TreeNode<'_>, String)> =
        vec![(tree.root(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, child) in dir.children() {
            let path = alloc::format!("{prefix}/{name}");
            if let Some(target) = first.get(&child.id()) {
                out.add_hard_link(&path, target)?;
                continue;
            }
            match child.kind() {
                NodeKind::Dir => {
                    out.add_dir(&path)?;
                    pending.push((child, path.clone()));
                }
                NodeKind::File(content) => {
                    let stored = match iso.extent_of(&path) {
                        Some(extent) => {
                            end = end.max((extent.offset() + extent.len()).div_ceil(2048));
                            Content::stored(vec![extent])
                        }
                        None if content.len().is_none_or(|len| len == 0) => Content::empty(),
                        None => {
                            let err = Detail::MissingExtent
                                .error::<core::convert::Infallible>(ErrorKind::Corrupt);
                            return Err(PathError::from(err).with_path(path));
                        }
                    };
                    out.add_file(&path, stored)?;
                }
                NodeKind::Symlink(target) => out.add_symlink(&path, target)?,
                NodeKind::Device(kind, number) => out.add_device(&path, kind, number)?,
                _ => continue,
            }
            out.set_metadata(&path, *child.metadata())?;
            if child.links() > 1 {
                first.insert(child.id(), path);
            }
        }
    }
    Ok((out, end))
}

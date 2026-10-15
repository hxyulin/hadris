//! Directory reads `FatFs` and the embedded `Fat` share.

use hadris_fat_raw::io::{BlockBuf, DirStart, DirWalk, Fat};
use hadris_fat_raw::{ENTRY_SIZE, RootLocation, Slot, short_name};
use hadris_fs::{CodePage, ErrorKind, FsResult};

use super::rawio;
use super::storage::BlockDevice;
use crate::names::CANDIDATES;

io_transform! {

/// Whether a short entry of `dir` other than the label and the slots
/// `skip(slot, offset)` covers has the on-disk name `name`.
pub(crate) async fn short_taken<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    dir: DirStart,
    name: &[u8; 11],
    skip: &impl Fn(u32, u64) -> bool,
) -> FsResult<bool, D::Error> {
    let mut walk = DirWalk::new(dir);
    let mut slot = 0;
    while let Some(offset) = rawio::slot_offset(dev, block, fat, &mut walk, slot).await? {
        match rawio::read_slot(dev, block, offset).await? {
            Slot::End => break,
            Slot::Short(entry) if entry.name() == *name && !entry.is_label() && !skip(slot, offset) => return Ok(true),
            _ => {}
        }
        slot += 1;
    }
    Ok(false)
}

/// A short name for `text` with a hashed `HHHH~N` tail that no entry of
/// `dir` has, as [`short_taken`] counts them.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn hashed_short<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    dir: DirStart,
    text: &str,
    code_page: &dyn CodePage,
    fold: fn(u16) -> u16,
    skip: &impl Fn(u32, u64) -> bool,
) -> FsResult<[u8; 11], D::Error> {
    for suffix in CANDIDATES as u8..=u8::MAX {
        let Some(mut candidate) = crate::names::short_name(text, suffix, code_page, fold) else {
            continue;
        };
        short_name::to_disk(&mut candidate);
        if !short_taken(dev, block, fat, dir, &candidate, skip).await? {
            return Ok(candidate);
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

/// The parent cluster recorded in the `..` entry of the directory at
/// `first`, or `None` for the root.
pub(crate) async fn dot_dot<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    first: u32,
) -> FsResult<Option<u32>, D::Error> {
    let offset = fat.cluster_at(first)? + ENTRY_SIZE as u64;
    let Slot::Short(entry) = rawio::read_slot(dev, block, offset).await? else {
        return Err(ErrorKind::Corrupt.into());
    };
    if !entry.is_dot_dot() {
        return Err(ErrorKind::Corrupt.into());
    }
    let cluster = entry.first_cluster(fat.geometry().kind());
    if cluster == 0 || fat.geometry().root() == RootLocation::Cluster(cluster) {
        return Ok(None);
    }
    Ok(Some(fat.check_cluster(cluster)?))
}

}

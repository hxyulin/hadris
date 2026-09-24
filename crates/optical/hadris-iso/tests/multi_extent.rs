//! Files of 4 GiB and more take several extents at Level 3 and are refused
//! below it.

mod common;

use std::collections::BTreeMap;

use hadris_fs::ErrorKind;
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{Content, Tree};
use hadris_io::ErrorType;
use hadris_iso::sync::IsoImage;
use hadris_iso::{IsoLevel, IsoOptions, Namespace};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, OutOfRange, WriteError};

const LEN: u64 = (1 << 32) + 4096;
static ZEROS: [u8; 2048] = [0; 2048];

/// Zeros except the last byte of every MiB, without holding them.
struct Sparse;

impl ErrorType for Sparse {
    type Error = std::convert::Infallible;
}

impl hadris_io::sync::ByteSource for Sparse {
    fn len(&self) -> u64 {
        LEN
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min((LEN - offset.min(LEN)) as usize);
        buf[..n].fill(0);
        let mut mark = (offset >> 20 << 20) + (1 << 20) - 1;
        while mark < offset + n as u64 {
            if mark >= offset {
                buf[(mark - offset) as usize] = (mark >> 20) as u8;
            }
            mark += 1 << 20;
        }
        Ok(n)
    }
}

/// A device that keeps only the blocks that are not all zero.
#[derive(Default)]
struct SparseDevice {
    blocks: BTreeMap<u64, Vec<u8>>,
}

impl ErrorType for SparseDevice {
    type Error = OutOfRange;
}

impl BlockDevice for SparseDevice {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }

    fn block_count(&self) -> u64 {
        (LEN >> 11) + 4096
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        for (i, chunk) in buf.chunks_mut(2048).enumerate() {
            match self.blocks.get(&(first.get() + i as u64)) {
                Some(data) => chunk.copy_from_slice(data),
                None => chunk.fill(0),
            }
        }
        Ok(())
    }

    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
        for (i, chunk) in buf.chunks(2048).enumerate() {
            let block = first.get() + i as u64;
            if chunk == &ZEROS[..chunk.len()] {
                self.blocks.remove(&block);
            } else {
                self.blocks.insert(block, chunk.to_vec());
            }
        }
        Ok(())
    }
}

#[test]
fn large_files_take_several_extents_at_level_3() {
    let mut tree = Tree::new();
    tree.add_file("big.bin", Content::source(Sparse)).unwrap();
    tree.add_file("small.txt", Content::bytes("after")).unwrap();
    let refused = hadris_iso::sync::plan(&tree, &IsoOptions::default()).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::FileTooLarge);

    let options = IsoOptions::default().with_level(IsoLevel::L3);
    let mut dev = SparseDevice::default();
    let report = hadris_iso::sync::write(&mut dev, &tree, &options).unwrap();
    assert_eq!(report.extent_of("big.bin").unwrap().len(), LEN);

    let mut iso = IsoImage::open(dev).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let node = view.resolve("/BIG.BIN").unwrap();
    assert_eq!(view.node_metadata(node).unwrap().len(), LEN);
    let mut extents = Vec::new();
    view.extents(node, |extent| extents.push(extent)).unwrap();
    assert_eq!(extents.len(), 2);
    assert_eq!(extents[0].end(), extents[1].offset());
    let mut byte = [0u8; 1];
    for mib in [0u64, 4095, 4096] {
        let at = (mib << 20) + (1 << 20) - 1;
        if at < LEN {
            view.read_at(node, at, &mut byte).unwrap();
            assert_eq!(byte[0], mib as u8, "MiB {mib}");
        }
    }
    let mut tail = [0xFFu8; 8192];
    let n = view.read_at(node, LEN - 4096, &mut tail).unwrap();
    assert_eq!(n, 4096);
    assert_eq!(view.read_to_vec("/SMALL.TXT").unwrap(), b"after");
    hadris_fs::sync::contract::check_read_only(&mut view).unwrap();
}

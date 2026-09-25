//! Files of 4 GiB and more take several extents at Level 3 and are refused
//! below it.

mod common;

use hadris_fs::MountOptions;
use std::collections::BTreeMap;

use common::{IsoExtras, Paths};
use hadris_fs::ErrorKind;
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Node, Tree};
use hadris_io::ErrorType;
use hadris_iso::sync::IsoFs;
use hadris_iso::{IsoLevel, IsoOptions, Namespace};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

const LEN: u64 = (1 << 32) + 4096;
static ZEROS: [u8; 2048] = [0; 2048];

/// A sparse host file of zeros except the last byte of every MiB.
fn sparse() -> tempfile::NamedTempFile {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(LEN).unwrap();
    let mut mark = (1u64 << 20) - 1;
    while mark < LEN {
        file.seek(SeekFrom::Start(mark)).unwrap();
        file.write_all(&[(mark >> 20) as u8]).unwrap();
        mark += 1 << 20;
    }
    file
}

/// A device that keeps only the blocks that are not all zero.
#[derive(Default)]
struct SparseDevice {
    blocks: BTreeMap<u64, Vec<u8>>,
}

impl ErrorType for SparseDevice {
    type Error = std::convert::Infallible;
}

impl BlockDevice for SparseDevice {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }

    fn block_count(&self) -> u64 {
        (LEN >> 11) + 4096
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
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
    ) -> Result<(), hadris_io::Error<Self::Error>> {
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

/// Windows fills a file with zeros up to each write, so the sparse input
/// would take 4 GiB of disk and minutes there.
#[test]
#[cfg_attr(windows, ignore)]
fn large_files_take_several_extents_at_level_3() {
    let file = sparse();
    let mut tree = Tree::new();
    tree.insert(
        "big.bin",
        Node::file(hadris_fs::host::file(file.path()).unwrap()),
    )
    .unwrap();
    tree.insert("small.txt", Node::file(Content::bytes("after")))
        .unwrap();
    let refused = hadris_iso::plan(&tree, &IsoOptions::default()).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::FileTooLarge);

    let options = IsoOptions::default().with_level(IsoLevel::L3);
    let mut dev = SparseDevice::default();
    let report = hadris_iso::sync::write(&mut dev, &tree, &options).unwrap();
    let written: Vec<_> = report.extents("big.bin").unwrap().to_vec();
    assert_eq!(written.len(), 2);
    assert_eq!(written.iter().map(|e| e.len()).sum::<u64>(), LEN);

    let mut iso = dev;
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    let node = view.resolve_path("/BIG.BIN").unwrap();
    assert_eq!(view.stat(node).unwrap().len(), LEN);
    let extents = view.all_extents(node);
    assert_eq!(extents.len(), 2);
    assert_eq!(extents[0].end(), extents[1].offset());
    let mut byte = [0u8; 1];
    for mib in [0u64, 4095, 4096] {
        let at = (mib << 20) + (1 << 20) - 1;
        if at < LEN {
            view.read(node, at, &mut byte).unwrap();
            assert_eq!(byte[0], mib as u8, "MiB {mib}");
        }
    }
    let mut tail = [0xFFu8; 8192];
    let n = view.read(node, LEN - 4096, &mut tail).unwrap();
    assert_eq!(n, 4096);
    assert_eq!(view.read_to_vec("/SMALL.TXT").unwrap(), b"after");
    hadris_fs::sync::contract::check_read_only(&mut view).unwrap();
}

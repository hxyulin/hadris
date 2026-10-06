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

#[test]
fn listing_the_end_of_a_4_gib_directory_stops() {
    let mut tree = Tree::new();
    tree.insert("a.txt", Node::file(Content::bytes("a")))
        .unwrap();
    let mut dev = SparseDevice::default();
    hadris_iso::sync::write(&mut dev, &tree, &IsoOptions::default()).unwrap();
    let mut pvd = [0u8; 2048];
    dev.read_blocks(BlockIndex::new(16), &mut pvd).unwrap();
    let root = u64::from(u32::from_le_bytes(pvd[158..162].try_into().unwrap()));
    let mut dot = [0u8; 2048];
    dev.read_blocks(BlockIndex::new(root), &mut dot).unwrap();
    dot[10..18].copy_from_slice(&[0xFF; 8]);
    dev.write_blocks(BlockIndex::new(root), &dot).unwrap();
    let last = root * 2048 + (1 << 32) - 2048;
    let mut block = [0u8; 2048];
    let record = hadris_iso::raw::DirectoryRecord::new(b"A", &[0; 76]).unwrap();
    assert_eq!(record.len(), 110);
    block[2048 - 110..].copy_from_slice(record.as_bytes());
    dev.write_blocks(BlockIndex::new(last / 2048), &block)
        .unwrap();

    let mut view =
        IsoFs::mount_namespace(&mut dev, MountOptions::new(), Namespace::Primary).unwrap();
    let root = view.root();
    let padding = hadris_fs::DirCursor::from_raw(u64::from(u32::MAX) - 2048);
    assert!(view.readdir(root, padding).unwrap().is_none());
    let past = hadris_fs::DirCursor::from_raw((1 << 32) - 110);
    assert_eq!(
        view.readdir(root, past).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
}

fn record_chain_image(mismatched: bool) -> (Vec<u8>, hadris_fs::NodeId) {
    use hadris_iso::raw::{DirectoryRecord, FileFlags, U16Both, U32Both};

    fn record(name: &[u8], block: u32, len: u32, flags: FileFlags) -> DirectoryRecord {
        let mut record = DirectoryRecord::new(name, &[]).unwrap();
        let header = record.header_mut();
        header.extent = U32Both::new(block);
        header.data_len = U32Both::new(len);
        header.volume_sequence_number = U16Both::new(1);
        header.flags = flags.bits();
        record
    }

    let mut bytes = vec![0; 64 * 2048];
    let dot = record(&[0], 20, 4096, FileFlags::DIRECTORY);
    let dotdot = record(&[1], 20, 4096, FileFlags::DIRECTORY);
    let pvd = &mut bytes[16 * 2048..17 * 2048];
    pvd[..7].copy_from_slice(b"\x01CD001\x01");
    pvd[80..88].copy_from_slice(bytemuck::bytes_of(&U32Both::new(64)));
    pvd[120..124].copy_from_slice(bytemuck::bytes_of(&U16Both::new(1)));
    pvd[124..128].copy_from_slice(bytemuck::bytes_of(&U16Both::new(1)));
    pvd[128..132].copy_from_slice(bytemuck::bytes_of(&U16Both::new(2048)));
    pvd[156..190].copy_from_slice(dot.as_bytes());
    bytes[17 * 2048..17 * 2048 + 7].copy_from_slice(b"\xffCD001\x01");
    let root = 20 * 2048;
    bytes[root..root + dot.len()].copy_from_slice(dot.as_bytes());
    bytes[root + dot.len()..root + dot.len() + dotdot.len()].copy_from_slice(dotdot.as_bytes());
    let first = root + dot.len() + dotdot.len();
    let head = record(b"CHAIN.BIN;1", 40, 3, FileFlags::NOT_FINAL);
    bytes[first..first + head.len()].copy_from_slice(head.as_bytes());
    let name = if mismatched {
        &b"OTHER.BIN;1"[..]
    } else {
        &b"CHAIN.BIN;1"[..]
    };
    let empty = record(name, 41, 0, FileFlags::NOT_FINAL);
    let tail = record(name, 41, 5, FileFlags::empty());
    let next = 21 * 2048;
    bytes[next..next + empty.len()].copy_from_slice(empty.as_bytes());
    bytes[next + empty.len()..next + empty.len() + tail.len()].copy_from_slice(tail.as_bytes());
    bytes[40 * 2048..40 * 2048 + 3].copy_from_slice(b"abc");
    bytes[41 * 2048..41 * 2048 + 5].copy_from_slice(b"defgh");
    (bytes, hadris_fs::NodeId::new(first as u64).unwrap())
}

macro_rules! chain_use_cases {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                #[allow(unused_imports)]
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DirCursor, Extent};
                use hadris_iso::Detail;
                use hadris_iso::$mode::IsoFs;
                use hadris_storage::MemDevice;

                let (bytes, node) = record_chain_image(false);
                let mut iso =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                assert_eq!(iso.stat(node).await.unwrap().len(), 8);
                let mut buf = [0; 8];
                assert_eq!(iso.read(node, 0, &mut buf).await.unwrap(), 3);
                assert_eq!(&buf[..3], b"abc");
                assert_eq!(iso.read(node, 3, &mut buf).await.unwrap(), 5);
                assert_eq!(&buf[..5], b"defgh");
                assert_eq!(iso.read(node, 8, &mut buf).await.unwrap(), 0);

                let mut extents = [Extent::new(0, 0); 3];
                assert_eq!(iso.extents(node, 0, &mut extents).await.unwrap(), 2);
                assert_eq!(extents[0], Extent::new(40 * 2048, 3));
                assert_eq!(extents[1], Extent::new(41 * 2048, 5).with_file_offset(3));
                assert_eq!(iso.extents(node, 3, &mut extents[..1]).await.unwrap(), 1);
                assert_eq!(extents[0], Extent::new(41 * 2048, 5).with_file_offset(3));
                assert_eq!(iso.records(node, &mut extents).await.unwrap(), 3);
                assert_eq!(extents[0].offset(), node.get());
                assert_eq!(extents[1].offset(), 21 * 2048);
                assert_eq!(extents[2].offset(), 21 * 2048 + extents[1].len());
                assert_eq!(
                    iso.records(node, &mut []).await.unwrap_err().kind(),
                    ErrorKind::LimitExceeded
                );

                let entry = iso
                    .readdir(iso.root(), DirCursor::START)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(entry.node(), node);
                assert!(
                    iso.readdir(iso.root(), entry.next_cursor())
                        .await
                        .unwrap()
                        .is_none()
                );

                let (bytes, node) = record_chain_image(true);
                let mut iso =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                assert_eq!(iso.read(node, 0, &mut buf).await.unwrap(), 3);
                for error in [
                    iso.stat(node).await.unwrap_err(),
                    iso.read(node, 3, &mut buf).await.unwrap_err(),
                    iso.extents(node, 0, &mut extents).await.unwrap_err(),
                    iso.records(node, &mut extents).await.unwrap_err(),
                    iso.readdir(iso.root(), DirCursor::START).await.unwrap_err(),
                ] {
                    assert_eq!(error.kind(), ErrorKind::Corrupt);
                    assert_eq!(Detail::of(&error), Some(Detail::MultiExtent));
                }
            });
        }
    };
}

macro_rules! sync_case {
    ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) };
}

#[cfg(feature = "async")]
macro_rules! async_case {
    ($body:expr) => {
        common::block_on($body)
    };
}

chain_use_cases!(
    sync,
    record_chains_share_validation_and_skip_sector_padding,
    sync_case
);

#[cfg(feature = "async")]
chain_use_cases!(
    r#async,
    async_record_chains_share_validation_and_skip_sector_padding,
    async_case
);

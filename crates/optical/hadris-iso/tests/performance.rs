mod common;

use hadris_fs::{Content, Node, Tree};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct Counted {
    inner: MemDevice<Vec<u8>>,
    reads: Arc<AtomicU64>,
}
impl hadris_io::ErrorType for Counted {
    type Error = core::convert::Infallible;
}
impl hadris_storage::sync::BlockDevice for Counted {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::read_blocks(&mut self.inner, first, buf)?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[cfg(feature = "async")]
impl hadris_storage::r#async::BlockDevice for Counted {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::read_blocks(self, first, buf)
    }
}
fn linked() -> Tree {
    let mut tree = Tree::new();
    for i in 0..128 {
        let name = format!("f{i:04}.txt");
        tree.insert(&name, Node::file(Content::bytes([i as u8; 16])))
            .unwrap();
        tree.link(&name, format!("l{i:04}.txt")).unwrap();
    }
    tree
}
macro_rules! cases {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{ErrorKind, MountOptions, Name};
                use hadris_iso::IsoOptions;
                use hadris_iso::$mode::IsoFs;
                let reads = Arc::new(AtomicU64::new(0));
                let dev = Counted {
                    inner: common::image(&linked(), &IsoOptions::new().with_rock_ridge()),
                    reads: reads.clone(),
                };
                let mut fs = IsoFs::mount(dev, MountOptions::new()).await.unwrap();
                let root = fs.root();
                reads.store(0, Ordering::Relaxed);
                assert_eq!(
                    fs.lookup(root, Name::new("absent"))
                        .await
                        .unwrap_err()
                        .kind(),
                    ErrorKind::NotFound
                );
                assert!(
                    reads.load(Ordering::Relaxed) <= 32,
                    "a miss must not resolve hard-link IDs"
                );
                let mut cursor = hadris_fs::DirCursor::START;
                while let Some(entry) = fs.readdir(root, cursor).await.unwrap() {
                    assert_eq!(*entry.metadata(), fs.stat(entry.node()).await.unwrap());
                    cursor = entry.next_cursor();
                }
                for i in [0, 63, 127] {
                    let original = fs
                        .lookup(root, Name::new(format!("f{i:04}.txt").as_bytes()))
                        .await
                        .unwrap();
                    let alias = fs
                        .lookup(root, Name::new(format!("l{i:04}.txt").as_bytes()))
                        .await
                        .unwrap();
                    assert_eq!(original, alias);
                    let mut data = [0; 16];
                    assert_eq!(fs.read(alias, 0, &mut data).await.unwrap(), 16);
                    assert_eq!(data, [i as u8; 16]);
                    assert_eq!(fs.stat(alias).await.unwrap().nlink(), 2);
                }
                let mut plain = Tree::new();
                for i in 0..128 {
                    plain
                        .insert(format!("f{i:04}.txt"), Node::file(Content::bytes([42; 16])))
                        .unwrap();
                }
                let dev = Counted {
                    inner: common::image(&plain, &IsoOptions::new().with_rock_ridge()),
                    reads: reads.clone(),
                };
                let mut fs = IsoFs::mount(dev, MountOptions::new()).await.unwrap();
                reads.store(0, Ordering::Relaxed);
                let mut cursor = hadris_fs::DirCursor::START;
                let mut count = 0;
                while let Some(entry) = fs.readdir(fs.root(), cursor).await.unwrap() {
                    assert_eq!(entry.metadata().len(), 16);
                    cursor = entry.next_cursor();
                    count += 1;
                }
                assert_eq!(count, 128);
                assert!(
                    reads.load(Ordering::Relaxed) < 300,
                    "listing should reuse the parsed file record"
                );
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
cases!(sync, lookup_defers_link_identity_sync, sync_case);
#[cfg(feature = "async")]
cases!(r#async, lookup_defers_link_identity_async, async_case);

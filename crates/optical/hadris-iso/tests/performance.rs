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
    type State = ();
    fn cancel(&mut self, _: &mut ()) {}
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut core::task::Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> core::task::Poll<Result<(), hadris_io::Error<Self::Error>>> {
        core::task::Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
            self, first, buf,
        ))
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
                #[allow(unused_imports)]
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
                #[cfg(feature = "cache")]
                for capacity in [1, 512] {
                    let options = hadris_iso::CacheOptions::new()
                        .with_blocks(32)
                        .with_records(256)
                        .with_links(capacity);
                    let dev = Counted {
                        inner: common::image(&linked(), &IsoOptions::new().with_rock_ridge()),
                        reads: reads.clone(),
                    };
                    let mut indexed = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(options);
                    reads.store(0, Ordering::Relaxed);
                    for i in 0..128 {
                        let a = indexed
                            .lookup(indexed.root(), Name::new(format!("f{i:04}.txt").as_bytes()))
                            .await
                            .unwrap();
                        let b = indexed
                            .lookup(indexed.root(), Name::new(format!("l{i:04}.txt").as_bytes()))
                            .await
                            .unwrap();
                        assert_eq!(a, b);
                    }
                    if capacity == 512 {
                        assert!(reads.load(Ordering::Relaxed) <= 32);
                    }
                    let mut cursor = hadris_fs::DirCursor::START;
                    let mut count = 0;
                    while let Some(entry) = indexed.readdir(indexed.root(), cursor).await.unwrap() {
                        assert_eq!(*entry.metadata(), indexed.stat(entry.node()).await.unwrap());
                        cursor = entry.next_cursor();
                        count += 1;
                    }
                    assert_eq!(count, 256);
                    indexed.clear_cache();
                    let a = indexed
                        .lookup(indexed.root(), Name::new("f0127.txt"))
                        .await
                        .unwrap();
                    assert_eq!(
                        a,
                        indexed
                            .lookup(indexed.root(), Name::new("l0127.txt"))
                            .await
                            .unwrap()
                    );
                }
                #[cfg(feature = "cache")]
                {
                    let mut image =
                        common::image(&linked(), &IsoOptions::new().with_rock_ridge()).into_inner();
                    for at in 0..image.len() - 44 {
                        if image[at..at + 4] == *b"PX\x2c\x01" {
                            image[at + 36..at + 44].fill(0);
                        }
                    }
                    let dev = Counted {
                        inner: MemDevice::new(image, common::SECTOR),
                        reads: reads.clone(),
                    };
                    let mut indexed = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(hadris_iso::CacheOptions::new().with_links(512));
                    let a = indexed
                        .lookup(indexed.root(), Name::new("f0127.txt"))
                        .await
                        .unwrap();
                    assert_eq!(
                        a,
                        indexed
                            .lookup(indexed.root(), Name::new("l0127.txt"))
                            .await
                            .unwrap()
                    );
                }
                #[cfg(feature = "cache")]
                {
                    let mut tree = Tree::new();
                    tree.insert("a", Node::file(Content::bytes("a"))).unwrap();
                    tree.link("a", "z").unwrap();
                    tree.insert("b", Node::file(Content::bytes("b"))).unwrap();
                    let bytes =
                        common::image(&tree, &IsoOptions::new().with_rock_ridge()).into_inner();
                    let dev = Counted {
                        inner: MemDevice::new(bytes.clone(), common::SECTOR),
                        reads: reads.clone(),
                    };
                    let mut probe = IsoFs::mount(dev, MountOptions::new()).await.unwrap();
                    let a = probe.lookup(probe.root(), Name::new("a")).await.unwrap();
                    let b = probe.lookup(probe.root(), Name::new("b")).await.unwrap();
                    let mut bad = bytes;
                    let start = b.get() as usize;
                    let px = (start..start + bad[start] as usize - 4)
                        .find(|&at| bad[at..at + 4] == *b"PX\x2c\x01")
                        .unwrap();
                    bad[px + 2] = 4;
                    let dev = Counted {
                        inner: MemDevice::new(bad, common::SECTOR),
                        reads: reads.clone(),
                    };
                    let mut indexed = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(hadris_iso::CacheOptions::new().with_links(8));
                    assert_eq!(
                        indexed
                            .lookup(indexed.root(), Name::new("a"))
                            .await
                            .unwrap(),
                        a
                    );
                    assert_eq!(
                        indexed
                            .lookup(indexed.root(), Name::new("b"))
                            .await
                            .unwrap_err()
                            .kind(),
                        ErrorKind::Corrupt
                    );
                    assert_eq!(
                        indexed
                            .lookup(indexed.root(), Name::new("a"))
                            .await
                            .unwrap(),
                        a
                    );
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
                #[cfg(feature = "cache")]
                {
                    let dev = Counted {
                        inner: common::image(&plain, &IsoOptions::new().with_rock_ridge()),
                        reads: reads.clone(),
                    };
                    let mut fs = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(hadris_iso::CacheOptions::new());
                    reads.store(0, Ordering::Relaxed);
                    let mut cursor = hadris_fs::DirCursor::START;
                    let mut count = 0;
                    while let Some(entry) = fs.readdir(fs.root(), cursor).await.unwrap() {
                        cursor = entry.next_cursor();
                        count += 1;
                    }
                    assert_eq!(count, 128);
                    assert!(reads.load(Ordering::Relaxed) <= 16);
                    let node = fs.lookup(fs.root(), Name::new("f0000.txt")).await.unwrap();
                    fs.stat(node).await.unwrap();
                    reads.store(0, Ordering::Relaxed);
                    for _ in 0..10 {
                        assert_eq!(fs.stat(node).await.unwrap().len(), 16);
                    }
                    assert_eq!(reads.load(Ordering::Relaxed), 0);
                    fs.clear_cache();
                    fs.stat(node).await.unwrap();
                    assert!(reads.load(Ordering::Relaxed) > 0);
                    let mut tree = Tree::new();
                    tree.insert("big.bin", Node::file(Content::bytes(vec![42; 1024 * 1024])))
                        .unwrap();
                    let dev = Counted {
                        inner: common::image(&tree, &IsoOptions::new()),
                        reads: reads.clone(),
                    };
                    let mut fs = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(hadris_iso::CacheOptions::new());
                    let node = fs.lookup(fs.root(), Name::new("BIG.BIN")).await.unwrap();
                    reads.store(0, Ordering::Relaxed);
                    let mut buf = [0; 4096];
                    for pos in (0..1024 * 1024).step_by(buf.len()) {
                        assert_eq!(fs.read(node, pos as u64, &mut buf).await.unwrap(), 4096);
                        assert_eq!(buf, [42; 4096]);
                    }
                    assert!(reads.load(Ordering::Relaxed) <= 257);
                    let options = hadris_iso::CacheOptions::new()
                        .with_blocks(0)
                        .with_records(1);
                    let dev = Counted {
                        inner: common::image(&plain, &IsoOptions::new()),
                        reads: reads.clone(),
                    };
                    let mut fs = IsoFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap()
                        .with_cache(options);
                    let a = fs.lookup(fs.root(), Name::new("F0000.TXT")).await.unwrap();
                    let b = fs.lookup(fs.root(), Name::new("F0001.TXT")).await.unwrap();
                    fs.stat(a).await.unwrap();
                    fs.stat(b).await.unwrap();
                    reads.store(0, Ordering::Relaxed);
                    fs.stat(a).await.unwrap();
                    assert_eq!(reads.load(Ordering::Relaxed), 1);
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
cases!(sync, lookup_defers_link_identity_sync, sync_case);
#[cfg(feature = "async")]
cases!(r#async, lookup_defers_link_identity_async, async_case);

#[cfg(all(feature = "cache", feature = "async"))]
#[test]
fn cancelled_cache_miss_is_not_retained() {
    use hadris_fs::{MountOptions, Name};
    use hadris_storage::r#async::BlockDevice;
    use std::sync::atomic::AtomicBool;
    struct Paused {
        inner: Counted,
        paused: Arc<AtomicBool>,
        failed: Arc<AtomicBool>,
    }
    impl hadris_io::ErrorType for Paused {
        type Error = core::convert::Infallible;
    }
    impl BlockDevice for Paused {
        type State = ();
        fn cancel(&mut self, _: &mut ()) {}
        fn block_size(&self) -> BlockSize {
            common::SECTOR
        }
        fn block_count(&self) -> u64 {
            hadris_storage::sync::BlockDevice::block_count(&self.inner)
        }
        fn poll_read_blocks(
            &mut self,
            _: &mut (),
            _: &mut core::task::Context<'_>,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> core::task::Poll<Result<(), hadris_io::Error<Self::Error>>> {
            if self.paused.load(Ordering::Relaxed) {
                return core::task::Poll::Pending;
            }
            core::task::Poll::Ready(if self.failed.load(Ordering::Relaxed) {
                Err(hadris_io::Error::new(
                    hadris_io::ErrorKind::Io,
                    "injected failure",
                ))
            } else {
                hadris_storage::sync::BlockDevice::read_blocks(&mut self.inner, first, buf)
            })
        }
    }

    let mut tree = Tree::new();
    tree.insert("file", Node::file(Content::bytes("data")))
        .unwrap();
    let reads = Arc::new(AtomicU64::new(0));
    let paused = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let dev = Paused {
        inner: Counted {
            inner: common::image(&tree, &hadris_iso::IsoOptions::new()),
            reads: reads.clone(),
        },
        paused: paused.clone(),
        failed: failed.clone(),
    };
    let mut fs = common::block_on(hadris_iso::r#async::IsoFs::mount(dev, MountOptions::new()))
        .unwrap()
        .with_cache(hadris_iso::CacheOptions::new());
    let node = common::block_on(fs.lookup(fs.root(), Name::new("FILE"))).unwrap();
    fs.clear_cache();
    reads.store(0, Ordering::Relaxed);
    paused.store(true, Ordering::Relaxed);
    {
        let mut future = core::pin::pin!(fs.stat(node));
        fn send<T: Send>(_: &T) {}
        send(&future);
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        assert!(core::future::Future::poll(future.as_mut(), &mut cx).is_pending());
    }
    paused.store(false, Ordering::Relaxed);
    assert_eq!(common::block_on(fs.stat(node)).unwrap().len(), 4);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    common::block_on(fs.stat(node)).unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    fs.clear_cache();
    failed.store(true, Ordering::Relaxed);
    assert_eq!(
        common::block_on(fs.stat(node)).unwrap_err().kind(),
        hadris_fs::ErrorKind::Io
    );
    failed.store(false, Ordering::Relaxed);
    assert_eq!(common::block_on(fs.stat(node)).unwrap().len(), 4);

    tree.link("file", "alias").unwrap();
    let dev = Paused {
        inner: Counted {
            inner: common::image(&tree, &hadris_iso::IsoOptions::new().with_rock_ridge()),
            reads: reads.clone(),
        },
        paused: paused.clone(),
        failed: failed.clone(),
    };
    let mut fs = common::block_on(hadris_iso::r#async::IsoFs::mount(dev, MountOptions::new()))
        .unwrap()
        .with_cache(hadris_iso::CacheOptions::new().with_links(8));
    common::block_on(fs.stat(fs.root())).unwrap();
    paused.store(true, Ordering::Relaxed);
    {
        let root = fs.root();
        let mut future = core::pin::pin!(fs.lookup(root, Name::new("file")));
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        assert!(core::future::Future::poll(future.as_mut(), &mut cx).is_pending());
    }
    paused.store(false, Ordering::Relaxed);
    let a = common::block_on(fs.lookup(fs.root(), Name::new("file"))).unwrap();
    assert_eq!(
        a,
        common::block_on(fs.lookup(fs.root(), Name::new("alias"))).unwrap()
    );
}

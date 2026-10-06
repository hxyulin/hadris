#[path = "common/fatfs.rs"]
mod common;

use hadris_fs::{Content, ErrorKind, Node, Tree};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

struct Counted {
    inner: MemDevice<Vec<u8>>,
    writes: usize,
    flushes: usize,
}
impl Counted {
    fn new(bytes: usize) -> Self {
        Self {
            inner: MemDevice::new(vec![0xA5; bytes], BlockSize::new(512).unwrap()),
            writes: 0,
            flushes: 0,
        }
    }
    fn untouched(&self) {
        assert_eq!(self.writes, 0);
        assert_eq!(self.flushes, 0);
        assert!(self.inner.get_ref().iter().all(|&byte| byte == 0xA5));
    }
}
impl hadris_io::ErrorType for Counted {
    type Error = core::convert::Infallible;
}
impl hadris_storage::sync::BlockDevice for Counted {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 512
    }
    fn writable(&self) -> bool {
        true
    }
    fn read_blocks(
        &mut self,
        first: BlockIndex,
        data: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.read_blocks(first, data)
    }
    fn write_blocks(
        &mut self,
        first: BlockIndex,
        data: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.writes += 1;
        self.inner.write_blocks(first, data)
    }
    fn flush(&mut self) -> Result<(), hadris_io::Error<Self::Error>> {
        self.flushes += 1;
        Ok(())
    }
}
#[cfg(feature = "async")]
impl hadris_storage::r#async::BlockDevice for Counted {
    fn block_size(&self) -> BlockSize {
        hadris_storage::sync::BlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(self)
    }
    fn writable(&self) -> bool {
        true
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        data: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::read_blocks(self, first, data)
    }
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        data: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::write_blocks(self, first, data)
    }
    async fn flush(&mut self) -> Result<(), hadris_io::Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::flush(self)
    }
}

macro_rules! cases {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fat::{FatKind, FatOptions};
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{MountOptions, Name, OpenMode};
                for (kind, size) in [
                    (FatKind::Fat12, 2 << 20),
                    (FatKind::Fat16, 8 << 20),
                    (FatKind::Fat32, 40 << 20),
                ] {
                    let options = FatOptions::new().with_kind(kind);
                    let mut empty = Counted::new(size);
                    let geo = hadris_fat::$mode::format(&mut empty, &options)
                        .await
                        .unwrap();
                    let free = u64::from(geo.max_cluster() - 1 - u32::from(kind == FatKind::Fat32));
                    let capacity = free * u64::from(geo.cluster_size());
                    for extra in [1, geo.cluster_size()] {
                        let mut tree = Tree::new();
                        tree.insert(
                            "DATA.BIN",
                            Node::file(Content::bytes(vec![
                                0;
                                (capacity + u64::from(extra)) as usize
                            ])),
                        )
                        .unwrap();
                        let mut out = Counted::new(size);
                        assert_eq!(
                            hadris_fat::$mode::write(&mut out, &tree, &options)
                                .await
                                .unwrap_err()
                                .kind(),
                            ErrorKind::NoSpace
                        );
                        out.untouched();
                    }
                    for directory in [false, true] {
                        let mut rejected = Tree::new();
                        let len = if directory {
                            capacity
                        } else {
                            capacity - u64::from(geo.cluster_size()) + 1
                        };
                        rejected
                            .insert(
                                "DATA.BIN",
                                Node::file(Content::bytes(vec![0; len as usize])),
                            )
                            .unwrap();
                        if directory {
                            rejected.insert("EMPTY", Node::dir()).unwrap();
                        } else {
                            rejected
                                .insert("TINY", Node::file(Content::bytes([0])))
                                .unwrap();
                        }
                        let mut out = Counted::new(size);
                        assert_eq!(
                            hadris_fat::$mode::write(&mut out, &rejected, &options)
                                .await
                                .unwrap_err()
                                .kind(),
                            ErrorKind::NoSpace
                        );
                        out.untouched();
                    }
                    let mut tree = Tree::new();
                    tree.insert(
                        "DATA.BIN",
                        Node::file(Content::bytes(vec![0x71; capacity as usize])),
                    )
                    .unwrap();
                    tree.link("DATA.BIN", "SKIPPED.BIN").unwrap();
                    tree.insert("link", Node::symlink("DATA.BIN")).unwrap();
                    let mut out = Counted::new(size);
                    hadris_fat::$mode::write(&mut out, &tree, &options)
                        .await
                        .unwrap();
                    let mut fs = hadris_fat::$mode::FatFs::mount(out, MountOptions::new())
                        .await
                        .unwrap();
                    let node = fs.lookup(fs.root(), Name::new("DATA.BIN")).await.unwrap();
                    assert_eq!(fs.stat(node).await.unwrap().len(), capacity);
                    fs.open(node, OpenMode::Read).await.unwrap();
                    let mut byte = [0];
                    assert_eq!(fs.read(node, capacity - 1, &mut byte).await.unwrap(), 1);
                    assert_eq!(byte, [0x71]);
                }
                let options = hadris_fat::exfat::ExFatOptions::new().with_fat_count(2);
                let size = 2 << 20;
                let mut empty = Counted::new(size);
                hadris_fat::exfat::$mode::format(&mut empty, &options)
                    .await
                    .unwrap();
                let mut fs = hadris_fat::exfat::$mode::ExFatFs::mount(empty, MountOptions::new())
                    .await
                    .unwrap();
                let stats = fs.statfs().await.unwrap();
                let capacity = stats.free_blocks() * u64::from(stats.block_size());
                let mut oversized = Tree::new();
                oversized
                    .insert(
                        "DATA.BIN",
                        Node::file(Content::bytes(vec![0; capacity as usize + 1])),
                    )
                    .unwrap();
                let mut out = Counted::new(size);
                assert_eq!(
                    hadris_fat::exfat::$mode::write(&mut out, &oversized, &options)
                        .await
                        .unwrap_err()
                        .kind(),
                    ErrorKind::NoSpace
                );
                out.untouched();
                let mut tree = Tree::new();
                tree.insert(
                    "DATA.BIN",
                    Node::file(Content::bytes(vec![0x71; capacity as usize])),
                )
                .unwrap();
                for index in 0..40 {
                    tree.insert(format!("F{index:07}"), Node::file(Content::empty()))
                        .unwrap();
                }
                tree.insert("OVERFLOW", Node::file(Content::empty()))
                    .unwrap();
                let mut rejected = Counted::new(size);
                assert_eq!(
                    hadris_fat::exfat::$mode::write(&mut rejected, &tree, &options)
                        .await
                        .unwrap_err()
                        .kind(),
                    ErrorKind::NoSpace
                );
                rejected.untouched();
                tree.remove("OVERFLOW").unwrap();
                let mut out = Counted::new(size);
                hadris_fat::exfat::$mode::write(&mut out, &tree, &options)
                    .await
                    .unwrap();
            });
        }
    };
}
macro_rules! sync_case { ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) }; }
#[cfg(feature = "async")]
macro_rules! async_case {
    ($body:expr) => {
        common::block_on($body)
    };
}
cases!(sync, capacity_boundaries_sync, sync_case);
#[cfg(feature = "async")]
cases!(r#async, capacity_boundaries_async, async_case);

#[test]
fn fixed_root_accounts_for_labels_and_long_names() {
    use hadris_fat::{FatKind, FatOptions, VolumeLabel};
    for kind in [FatKind::Fat12, FatKind::Fat16] {
        let size = if kind == FatKind::Fat12 {
            2 << 20
        } else {
            8 << 20
        };
        let options = FatOptions::new()
            .with_kind(kind)
            .with_root_entries(16)
            .with_label(VolumeLabel::new("LABEL").unwrap());
        let mut tree = Tree::new();
        for index in 0..15 {
            tree.insert(format!("F{index:07}"), Node::file(Content::empty()))
                .unwrap();
        }
        hadris_fat::sync::write(Counted::new(size), &tree, &options).unwrap();
        tree.insert("overflow", Node::file(Content::empty()))
            .unwrap();
        let mut out = Counted::new(size);
        assert_eq!(
            hadris_fat::sync::write(&mut out, &tree, &options)
                .unwrap_err()
                .kind(),
            ErrorKind::NoSpace
        );
        out.untouched();
        tree.remove("overflow").unwrap();
        tree.remove("F0000014").unwrap();
        tree.insert("Long filename.txt", Node::file(Content::empty()))
            .unwrap();
        let mut out = Counted::new(size);
        assert_eq!(
            hadris_fat::sync::write(&mut out, &tree, &options)
                .unwrap_err()
                .kind(),
            ErrorKind::NoSpace
        );
        out.untouched();
    }
}

#[test]
fn invalid_tree_names_do_not_format_the_destination() {
    let mut tree = Tree::new();
    tree.insert(b"bad\xFF", Node::file(Content::empty()))
        .unwrap();
    let mut out = Counted::new(64 << 10);
    assert_eq!(
        hadris_fat::sync::write(&mut out, &tree, &hadris_fat::FatOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    out.untouched();
}

#[test]
fn advertised_stream_lengths_are_preflighted_without_opening_content() {
    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(3 << 20).unwrap();
    let mut tree = Tree::new();
    tree.insert(
        "DATA.BIN",
        Node::file(hadris_fs::host::file(file.path()).unwrap()),
    )
    .unwrap();
    std::fs::remove_file(file.path()).unwrap();
    let mut out = Counted::new(2 << 20);
    assert_eq!(
        hadris_fat::sync::write(&mut out, &tree, &hadris_fat::FatOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::NoSpace
    );
    out.untouched();
}

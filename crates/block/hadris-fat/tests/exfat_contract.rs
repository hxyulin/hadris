//! `ExFatFs` passes the `hadris-fs` driver contract kit in the raw and
//! shared tiers and in the async modes, and the image is clean afterwards.

#[path = "common/exfat.rs"]
mod common;

use common::{block_on, clean, fsck};
use hadris_fat::exfat::{FormatOptions, MountOptions};
use hadris_fs::HeapTable;

fn blank(size: usize, cluster: u32) -> Vec<u8> {
    common::image(common::small(size, cluster))
}

#[test]
fn sync_raw_tier() {
    for (size, cluster) in [(4 << 20, 512), (16 << 20, 4096), (64 << 20, 32 << 10)] {
        let mut fs = common::mount(&blank(size, cluster));
        hadris_fs::sync::contract::check(&mut fs).unwrap_or_else(|err| panic!("{cluster}: {err}"));
        assert_eq!(fs.open_nodes(), 1);
        fs.sync().unwrap();
        clean(&mut fs, "contract");
        fsck(&common::image(fs), "contract");
    }
}

#[test]
fn sync_shared_tier() {
    let fs = hadris_fat::exfat::sync::ExFatFs::open_with(
        common::device(blank(8 << 20, 4096), 512),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap();
    let vol = hadris_fs::sync::Volume::new(fs);
    hadris_fs::sync::contract::check(&mut &vol).unwrap();
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[test]
fn async_modes() {
    block_on(async {
        let dev = common::device(vec![0u8; 8 << 20], 4096);
        let mut fs = hadris_fat::exfat::r#async::format(dev, FormatOptions::new())
            .await
            .unwrap();
        hadris_fs::r#async::contract::check(&mut fs).await.unwrap();
        let mut dev = fs.into_inner();
        let mut findings = Vec::new();
        hadris_fat::exfat::r#async::check(&mut dev, &mut [0u8; 1536], |f| {
            findings.push(common::Found::new(f))
        })
        .await
        .unwrap();
        assert!(
            findings
                .iter()
                .all(|f| f.detail == hadris_fat::exfat::Detail::Dirty),
            "{findings:?}"
        );
        let fs =
            hadris_fat::exfat::async_send::ExFatFs::open(common::device(blank(8 << 20, 4096), 512))
                .await
                .unwrap();
        let vol = hadris_fs::async_send::Volume::new(fs);
        hadris_fs::async_send::contract::check(&mut &vol)
            .await
            .unwrap();
    });
}

#[test]
fn every_mode_writes_the_same_bytes() {
    use hadris_fs::{Name, NewNode, SetMetadata};
    let name = |text| Name::new(text).unwrap();
    let sync = {
        let mut fs = hadris_fat::exfat::sync::format(
            common::device(vec![0u8; 4 << 20], 512),
            FormatOptions::new(),
        )
        .unwrap();
        let root = fs.root();
        let dir = fs
            .create(root, name("dir"), NewNode::Dir, &SetMetadata::new())
            .unwrap();
        let file = fs
            .create(dir, name("file.txt"), NewNode::File, &SetMetadata::new())
            .unwrap();
        fs.write_at(file, 0, &common::payload(9000, 3)).unwrap();
        fs.sync().unwrap();
        let mut dev = fs.into_inner();
        let (_, found) = common::check_dev(&mut dev, 4096);
        assert_eq!(found, []);
        dev.into_inner()
    };
    let send = block_on(async {
        use hadris_fat::exfat::async_send;
        let mut fs = async_send::format(
            common::device(vec![0u8; 4 << 20], 512),
            FormatOptions::new(),
        )
        .await
        .unwrap();
        let root = fs.root();
        let dir = fs
            .create(root, name("dir"), NewNode::Dir, &SetMetadata::new())
            .await
            .unwrap();
        let file = fs
            .create(dir, name("file.txt"), NewNode::File, &SetMetadata::new())
            .await
            .unwrap();
        fs.write_at(file, 0, &common::payload(9000, 3))
            .await
            .unwrap();
        fs.sync().await.unwrap();
        let mut dev = fs.into_inner();
        let report = async_send::check(&mut dev, &mut [0u8; 4096], |_| {})
            .await
            .unwrap();
        assert!(report.is_clean());
        dev.into_inner()
    });
    assert!(sync == send, "sync and async_send write the same image");
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn async_send_futures_are_send() {
    let mut fs = block_on(hadris_fat::exfat::async_send::ExFatFs::open(
        common::device(blank(4 << 20, 4096), 512),
    ))
    .unwrap();
    let root = fs.root();
    assert_send(&fs.stats());
    let _ = root;
    let mut dev = fs.into_inner();
    let mut scratch = [0u8; 1536];
    assert_send(&hadris_fat::exfat::async_send::check(
        &mut dev,
        &mut scratch,
        |_| {},
    ));
}

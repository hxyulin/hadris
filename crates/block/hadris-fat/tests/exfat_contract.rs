//! `ExFatFs` passes the `hadris-fs` driver contract kit directly and
//! through a `Volume`, in both modes, and the image is clean afterwards.

#[path = "common/exfat.rs"]
mod common;
use hadris_fs::MountOptions;
use hadris_fs::r#async::FileSystem as _;
use hadris_fs::sync::FileSystem;

use common::{block_on, clean, fsck};
use hadris_fat::exfat::FormatOptions;

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
fn sync_through_a_volume() {
    let fs = hadris_fat::exfat::sync::ExFatFs::mount(
        common::device(blank(8 << 20, 4096), 512),
        MountOptions::new(),
    )
    .unwrap();
    let vol = hadris_fs::sync::Volume::new(fs);
    hadris_fs::sync::contract::check(&mut *vol.lock()).unwrap();
    assert_eq!(vol.into_inner().unwrap().open_nodes(), 1);
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
        let fs = hadris_fat::exfat::r#async::ExFatFs::mount(
            common::device(blank(8 << 20, 4096), 512),
            MountOptions::new(),
        )
        .await
        .unwrap();
        let vol = hadris_fs::r#async::Volume::new(fs);
        hadris_fs::r#async::contract::check(&mut *vol.lock().await)
            .await
            .unwrap();
    });
}

#[test]
fn every_mode_writes_the_same_bytes() {
    use hadris_fs::{Name, SetAttr};
    let name = |text| Name::new(text);
    let sync = {
        let mut fs = hadris_fat::exfat::sync::format(
            common::device(vec![0u8; 4 << 20], 512),
            FormatOptions::new(),
        )
        .unwrap();
        let root = fs.root();
        let dir = fs.mkdir(root, name("dir"), &SetAttr::new()).unwrap();
        let file = fs.create(dir, name("file.txt"), &SetAttr::new()).unwrap();
        fs.write(file, 0, &common::payload(9000, 3)).unwrap();
        fs.sync().unwrap();
        let mut dev = fs.into_inner();
        let (_, found) = common::check_dev(&mut dev, 4096);
        assert_eq!(found, []);
        dev.into_inner()
    };
    let send = block_on(async {
        use hadris_fat::exfat::r#async as asynch;
        let mut fs = asynch::format(
            common::device(vec![0u8; 4 << 20], 512),
            FormatOptions::new(),
        )
        .await
        .unwrap();
        let root = fs.root();
        let dir = fs.mkdir(root, name("dir"), &SetAttr::new()).await.unwrap();
        let file = fs
            .create(dir, name("file.txt"), &SetAttr::new())
            .await
            .unwrap();
        fs.write(file, 0, &common::payload(9000, 3)).await.unwrap();
        fs.sync().await.unwrap();
        let mut dev = fs.into_inner();
        let report = asynch::check(&mut dev, &mut [0u8; 4096], |_| {})
            .await
            .unwrap();
        assert!(report.is_clean());
        dev.into_inner()
    });
    assert!(sync == send, "sync and async write the same image");
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn async_futures_are_send() {
    let mut fs = block_on(hadris_fat::exfat::r#async::ExFatFs::mount(
        common::device(blank(4 << 20, 4096), 512),
        MountOptions::new(),
    ))
    .unwrap();
    let root = fs.root();
    assert_send(&fs.statfs());
    let _ = root;
    let mut dev = fs.into_inner();
    let mut scratch = [0u8; 1536];
    assert_send(&hadris_fat::exfat::r#async::check(
        &mut dev,
        &mut scratch,
        |_| {},
    ));
}

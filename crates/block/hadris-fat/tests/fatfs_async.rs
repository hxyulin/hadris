//! `FatFs` in the `async` and `async_send` modes.

#[path = "common/fatfs.rs"]
mod common;

use std::sync::Arc;

use common::{CASES, INNER, INNER_FILES, LONG_NAME, block_on};
use hadris_fs::{
    DirCursor, ErrorKind, Name, NameBuf, NewNode, OpenOptions, RenameFlags, SetMetadata,
};

#[test]
fn async_mode_reads_through_every_tier() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::{DriverExt, PathExt, Volume};
    use hadris_io::r#async::Read as _;

    let case = CASES[2];
    block_on(async {
        let mut fs = FatFs::open(common::device(case, common::build(case)))
            .await
            .unwrap();
        let root = fs.root();
        let long = fs
            .lookup(root, Name::new("a LONG file name.TXT").unwrap())
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(fs.read_at(long, 4990, &mut buf).await.unwrap(), 10);
        assert_eq!(buf[..10], common::payload(5000, 1)[4990..]);
        fs.forget(long);

        let mut cursor = DirCursor::start();
        let mut name = NameBuf::new();
        let first = fs
            .read_dir_entry(root, &mut cursor, &mut name)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(name.as_bytes(), b"README.TXT");
        assert!(first.file_type().is_file());
        assert_eq!(
            fs.read_to_vec("/Nested Dir/sibling.txt").await.unwrap(),
            b"sibling"
        );

        let vol = Volume::new(fs);
        let mut dir = vol.read_dir(INNER).await.unwrap();
        let mut count = 0;
        while let Some(item) = dir.next_entry().await {
            item.unwrap();
            count += 1;
        }
        drop(dir);
        assert_eq!(count, INNER_FILES);
        let mut file = vol.open("/frag.bin", OpenOptions::read()).await.unwrap();
        let mut head = [0u8; 100];
        file.read_exact(&mut head).await.unwrap();
        assert_eq!(head[..], common::payload(100, 2)[..]);
        file.close().await.unwrap();
        assert_eq!(vol.into_inner().open_nodes(), 1);
    });
}

fn spawn_read<F: hadris_fs::async_send::FileSystem + 'static>(
    fs: Arc<F>,
    path: &'static str,
) -> std::thread::JoinHandle<Vec<u8>> {
    use hadris_fs::async_send::PathExt;
    std::thread::spawn(move || block_on(async move { fs.read_to_vec(path).await.unwrap() }))
}

#[test]
fn async_send_futures_move_to_other_threads() {
    use hadris_fat::async_send::FatFs;
    use hadris_fs::async_send::Volume;

    let case = CASES[1];
    let fs = block_on(FatFs::open(common::device(case, common::build(case)))).unwrap();
    let vol = Arc::new(Volume::new(fs));
    let deep = spawn_read(Arc::clone(&vol), "/nested dir/inner/deep.bin");
    let long = spawn_read(Arc::clone(&vol), "/A long file name.txt");
    assert_eq!(deep.join().unwrap(), common::payload(70_000, 5));
    assert_eq!(long.join().unwrap(), common::payload(5000, 1));

    let task = async {
        let mut fs = vol.lock().await;
        let root = fs.root();
        fs.lookup(root, Name::new(LONG_NAME).unwrap())
            .await
            .map(|node| fs.forget(node))
    };
    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    block_on(assert_send(task)).unwrap();
    let fs = Arc::into_inner(vol).unwrap().into_inner();
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn async_mode_writes() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::{DriverExt, PathExt, Volume};
    use hadris_io::r#async::Write as _;

    let case = CASES[0];
    let image = block_on(async {
        let mut fs = FatFs::open(common::device(case, common::blank(case)))
            .await
            .unwrap();
        let root = fs.root();
        let meta = SetMetadata::new();
        let dir = fs
            .create(root, Name::new("A Directory").unwrap(), NewNode::Dir, &meta)
            .await
            .unwrap();
        let file = fs
            .create(dir, Name::new("notes.txt").unwrap(), NewNode::File, &meta)
            .await
            .unwrap();
        let data = common::payload(9_000, 3);
        assert_eq!(fs.write_at(file, 0, &data).await.unwrap(), data.len());
        fs.set_len(file, 8_000).await.unwrap();
        fs.rename(
            dir,
            Name::new("notes.txt").unwrap(),
            root,
            Name::new("Renamed Notes.txt").unwrap(),
            RenameFlags::empty(),
        )
        .await
        .unwrap();
        assert_eq!(
            fs.remove(root, Name::new("A Directory").unwrap())
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Busy
        );
        fs.forget(dir);
        fs.remove(root, Name::new("a directory").unwrap())
            .await
            .unwrap();
        let mut buf = vec![0u8; 9_000];
        assert_eq!(fs.read_at(file, 0, &mut buf).await.unwrap(), 8_000);
        assert_eq!(buf[..8_000], data[..8_000]);
        fs.forget(file);
        fs.sync().await.unwrap();
        assert_eq!(fs.open_nodes(), 1);
        fs.write_file("/second.bin", b"second").await.unwrap();

        let vol = Volume::new(fs);
        let mut log = vol
            .open("/log.txt", OpenOptions::write().create().append())
            .await
            .unwrap();
        log.write_all(b"one ").await.unwrap();
        log.write_all(b"two").await.unwrap();
        log.close().await.unwrap();
        assert_eq!(vol.read_to_vec("/LOG.TXT").await.unwrap(), b"one two");
        let mut fs = vol.into_inner();
        fs.sync().await.unwrap();
        fs.into_inner().into_inner()
    });
    let v2 = common::open_v2(&image);
    let root = v2.root_dir();
    let read = |name: &str| {
        use hadris_fat::FatVolumeReadExt;
        let entry = root.find(name).unwrap().unwrap();
        v2.read_file(&entry).unwrap().read_to_vec().unwrap()
    };
    assert_eq!(read("Renamed Notes.txt"), common::payload(8_000, 3));
    assert_eq!(read("second.bin"), b"second");
    assert_eq!(read("log.txt"), b"one two");
    assert!(root.find("A Directory").unwrap().is_none());
}

#[test]
fn async_send_writers_on_other_threads() {
    use hadris_fat::async_send::FatFs;
    use hadris_fs::async_send::{FileSystem, PathExt, Volume};

    let case = CASES[2];
    let fs = block_on(FatFs::open(common::device(case, common::blank(case)))).unwrap();
    let vol = Arc::new(Volume::new(fs));
    let writers: Vec<_> = (0..4u8)
        .map(|i| {
            let vol = Arc::clone(&vol);
            std::thread::spawn(move || {
                block_on(async move {
                    let path = format!("/writer {i}.bin");
                    vol.write_file(&path, &common::payload(20_000, i))
                        .await
                        .unwrap();
                })
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    for i in 0..4u8 {
        let data = spawn_read(
            Arc::clone(&vol),
            [
                "/writer 0.bin",
                "/writer 1.bin",
                "/writer 2.bin",
                "/writer 3.bin",
            ][i as usize],
        );
        assert_eq!(data.join().unwrap(), common::payload(20_000, i));
    }
    block_on(vol.sync()).unwrap();
    let fs = Arc::into_inner(vol).unwrap().into_inner();
    assert_eq!(fs.open_nodes(), 1);
}

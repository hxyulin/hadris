//! `FatFs` in the `async` and `async_send` modes.

#[path = "common/fatfs.rs"]
mod common;

use std::sync::Arc;

use common::{CASES, INNER, INNER_FILES, LONG_NAME, block_on};
use hadris_fs::{DirCursor, Name, NameBuf, OpenOptions};

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

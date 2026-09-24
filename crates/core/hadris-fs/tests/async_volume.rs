//! The async mode runs the same jobs as the sync mode, and lets generic code
//! move its futures to other threads.

#![cfg(all(feature = "async", feature = "std"))]

mod common;

use std::sync::Arc;

use common::asynch::{MemFs, fixture};
use common::block_on;
use hadris_fs::r#async::{FileSystem, Volume, copy_tree};
use hadris_fs::{ErrorKind, FsResult, OpenOptions, Resolve};
use hadris_io::r#async::Write as _;

async fn read<F: FileSystem>(vol: &Volume<F>, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
    let mut file = vol.open(path, OpenOptions::new().read()).await?;
    let mut out = vec![0u8; 64];
    let n = file.read(&mut out).await?;
    out.truncate(n);
    file.close().await?;
    Ok(out)
}

#[test]
fn paths_handles_and_policies() {
    block_on(async {
        let vol = Volume::with_resolve(fixture(), Resolve::Follow);
        vol.create_dir_all("/d").await.unwrap();
        let mut file = vol
            .open("/d/log", OpenOptions::new().write().create().append())
            .await
            .unwrap();
        file.write_all(b"one ").await.unwrap();
        file.write_all(b"two").await.unwrap();
        file.close().await.unwrap();
        assert_eq!(read(&vol, "/d/log").await.unwrap(), b"one two");
        let mut dir = vol.read_dir("/d").await.unwrap();
        let entry = dir.next_entry().await.unwrap().unwrap();
        assert_eq!(entry.name().as_bytes(), b"log");
        assert!(dir.next_entry().await.is_none());
        drop(dir);
        assert_eq!(read(&vol, "/link/conf").await.unwrap(), b"key=value");
        assert_eq!(
            read(&vol, "/loop").await.unwrap_err().kind(),
            ErrorKind::Symlink
        );
        vol.remove_dir_all("/d").await.unwrap();
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn copy_tree_between_filesystems() {
    block_on(async {
        let mut src = MemFs::new();
        src.add("/", "etc", hadris_fs::FileType::Dir, b"");
        src.add("/etc", "conf", hadris_fs::FileType::File, b"key=value");
        let mut dst = MemFs::new();
        copy_tree(&mut src, "/etc", &mut dst, "/copy/etc")
            .await
            .unwrap();
        assert_eq!(dst.contents("/copy/etc/conf").unwrap(), b"key=value");
        let err = copy_tree(&mut src, "/nope", &mut dst, "/x")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
    });
}

#[test]
fn a_dropped_file_is_closed_by_the_next_call() {
    block_on(async {
        let vol = Volume::new(fixture());
        let before = vol.lock().await.closes();
        let mut file = vol
            .open("/log", OpenOptions::new().write().create())
            .await
            .unwrap();
        file.write_all(b"entry").await.unwrap();
        drop(file);
        assert_eq!(vol.lock().await.closes(), before + 1);
        let mut files = Vec::new();
        for _ in 0..40 {
            files.push(vol.open("/a.txt", OpenOptions::new().read()).await.unwrap());
        }
        let guard = vol.lock().await;
        drop(files);
        drop(guard);
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

/// Generic over the filesystem, with no `Send` bounds: the mode's
/// supertraits prove them.
fn spawn_read<F: FileSystem + 'static>(
    vol: Volume<F>,
    path: &'static str,
) -> std::thread::JoinHandle<FsResult<Vec<u8>, F::DeviceError>> {
    std::thread::spawn(move || block_on(async move { read(&vol, path).await }))
}

#[test]
fn generic_code_spawns_across_threads() {
    let vol = Volume::new(fixture());
    let a = spawn_read(vol.clone(), "/a.txt");
    let b = spawn_read(vol.clone(), "/etc/conf");
    assert_eq!(a.join().unwrap().unwrap(), b"root a");
    assert_eq!(b.join().unwrap().unwrap(), b"key=value");
    let fs = block_on(vol.into_inner()).unwrap();
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(t: T) -> T {
        t
    }
    let vol = Volume::new(fixture());
    let task = async {
        let mut file = vol.open("/a.txt", OpenOptions::new().read()).await.unwrap();
        let mut buf = [0u8; 4];
        file.read(&mut buf).await.unwrap();
        let mut guard = vol.lock().await;
        let root = guard.root();
        guard.stat(root).await.unwrap();
        buf
    };
    assert_eq!(&block_on(assert_send(task)), b"root");
    let src = Arc::new(());
    let copy = async move {
        let _keep = src;
        let mut a = MemFs::new();
        let mut b = MemFs::new();
        copy_tree(&mut a, "/", &mut b, "/x").await
    };
    block_on(assert_send(copy)).unwrap();
}

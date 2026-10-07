//! The async mode runs the same jobs as the sync mode, and lets generic code
//! move its futures to other threads.

#![cfg(all(feature = "async", feature = "std"))]

mod common;

use std::sync::Arc;

use common::asynch::{MemFs, fixture};
use common::block_on;
use hadris_fs::r#async::{ContentReader, FileSystem, Volume, copy_tree, read_tree};
use hadris_fs::{ErrorKind, FsResult, OpenOptions, Resolve, SetAttr};
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
fn volumes_copy_through_trees() {
    block_on(async {
        let mut src = MemFs::new();
        src.add("/", "etc", hadris_fs::FileType::Dir, b"");
        src.add("/etc", "conf", hadris_fs::FileType::File, b"key=value");
        let vol = Volume::new(src);
        let tree = read_tree(&vol, "/etc").await.unwrap();
        let mut dst = MemFs::new();
        let root = dst.root();
        copy_tree(&tree, &mut dst, root).await.unwrap();
        assert_eq!(dst.contents("/conf").unwrap(), b"key=value");
        let err = read_tree(&vol, "/nope").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        drop(tree);
        let src = vol.into_inner().await.ok().unwrap();
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

/// Polls `future` once, expecting it to wait, and drops it, as a caller that
/// gives up does.
fn cancel<F: core::future::Future>(future: F) {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    assert!(future.as_mut().poll(&mut context).is_pending());
}

#[test]
fn a_dropped_close_still_closes_the_file() {
    block_on(async {
        let vol = Volume::new(fixture());
        let file = vol.open("/a.txt", OpenOptions::new().read()).await.unwrap();
        let guard = vol.lock().await;
        cancel(file.close());
        drop(guard);
        vol.remove_file("/a.txt").await.unwrap();
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn a_close_dropped_while_the_driver_closes_still_closes_the_file() {
    block_on(async {
        let vol = Volume::new(fixture());
        let file = vol.open("/a.txt", OpenOptions::new().read()).await.unwrap();
        vol.lock().await.stall_in(0);
        cancel(file.close());
        vol.remove_file("/a.txt").await.unwrap();
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn an_open_dropped_while_closing_after_a_failed_truncate_leaves_no_open() {
    block_on(async {
        let vol = Volume::new(fixture());
        for calls in 0.. {
            let mut fs = vol.lock().await;
            fs.stall_in(calls);
            fs.fail_truncate();
            drop(fs);
            let pending = poll_once(vol.open("/a.txt", OpenOptions::new().write().truncate()));
            let mut fs = vol.lock().await;
            assert_eq!(fs.open_files(), 0, "stalled at call {calls}");
            fs.stall_in(u32::MAX);
            drop(fs);
            if !pending {
                break;
            }
        }
        vol.remove_file("/a.txt").await.unwrap();
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn dropped_path_calls_leave_no_pins() {
    block_on(async {
        let vol = Volume::new(fixture());
        vol.lock().await.stall_next();
        cancel(vol.metadata("/etc/conf"));
        vol.lock().await.stall_next();
        cancel(vol.read_dir("/etc"));
        vol.lock().await.stall_next();
        cancel(vol.remove_file("/etc/conf"));
        vol.lock().await.stall_next();
        cancel(vol.set_attr("/a.txt", &SetAttr::new()));
        vol.lock().await.stall_next();
        cancel(vol.create_dir_all("/x/y"));
        vol.lock().await.stall_next();
        cancel(vol.open("/a.txt", OpenOptions::new().write().truncate()));
        vol.remove_file("/a.txt").await.unwrap();
        assert_eq!(vol.lock().await.open_nodes(), 1);
        vol.lock().await.stall_next();
        cancel(vol.remove_dir_all("/etc"));
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

/// Polls `future` once and drops it. Whether it was still waiting.
fn poll_once<F: core::future::Future>(future: F) -> bool {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    future.as_mut().poll(&mut context).is_pending()
}

#[test]
fn dropped_resolves_leave_no_pins() {
    let mut fs = fixture();
    for how in [Resolve::Lexical, Resolve::Follow, Resolve::NoFollow] {
        for path in [
            "/etc/conf",
            "/link/conf",
            "/etc/up/conf",
            "/etc/../a.txt",
            "/abs",
        ] {
            for calls in 0.. {
                fs.stall_in(calls);
                let mut done = None;
                let pending =
                    poll_once(async { done = Some(fs.resolve(path.as_bytes(), how).await) });
                if let Some(Ok(node)) = done {
                    fs.forget(node, 1);
                }
                assert_eq!(fs.open_nodes(), 1, "{how:?} {path} stalled at call {calls}");
                if !pending {
                    break;
                }
            }
        }
    }
}

#[test]
fn dropped_tree_reads_leave_no_pins() {
    block_on(async {
        let vol = Volume::new(fixture());
        for calls in 0.. {
            vol.lock().await.stall_in(calls);
            let pending = poll_once(read_tree(&vol, "/"));
            let fs = vol.lock().await;
            assert_eq!(
                (fs.open_nodes(), fs.open_files()),
                (1, 0),
                "stalled at call {calls}"
            );
            drop(fs);
            if !pending {
                break;
            }
        }
        let tree = read_tree(&vol, "/").await.unwrap();
        let content = tree.get("/etc/conf").unwrap().content().unwrap();
        vol.lock().await.stall_in(1);
        assert!(poll_once(async {
            let mut reader = ContentReader::open(content).await.unwrap();
            reader.read_at(0, &mut [0u8; 4]).await.unwrap();
        }));
        assert_eq!(vol.lock().await.open_files(), 0);
        drop(tree);
        let fs = vol.into_inner().await.ok().unwrap();
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
    let src = Arc::new(hadris_fs::Tree::new());
    let copy = async move {
        let mut fs = MemFs::new();
        let root = fs.root();
        copy_tree(&src, &mut fs, root).await
    };
    block_on(assert_send(copy)).unwrap();
}

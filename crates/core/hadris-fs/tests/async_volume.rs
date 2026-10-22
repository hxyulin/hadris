//! The async mode runs the same jobs as the sync mode, and lets generic code
//! move its futures to other threads.

#![cfg(all(feature = "async", feature = "std"))]

mod common;

use std::sync::Arc;

use common::asynch::{MemFs, fixture};
use common::block_on;
use hadris_fs::async_::{
    ContentReader, FileSystem, SendVolume, Sendable, Volume, copy_tree, read_tree,
};
use hadris_fs::{ErrorKind, FsResult, OpenOptions, Resolve, SetAttr};
use hadris_io::async_::Write as _;

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
        let vol = SendVolume::new(Sendable::new(src));
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
        let before = vol.lock().await.unwrap().closes();
        let mut file = vol
            .open("/log", OpenOptions::new().write().create())
            .await
            .unwrap();
        file.write_all(b"entry").await.unwrap();
        drop(file);
        assert_eq!(vol.lock().await.unwrap().closes(), before + 1);
        let mut files = Vec::new();
        for _ in 0..40 {
            files.push(vol.open("/a.txt", OpenOptions::new().read()).await.unwrap());
        }
        let guard = vol.lock().await.unwrap();
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
        let guard = vol.lock().await.unwrap();
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
        vol.lock().await.unwrap().stall_in(0);
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
            let mut fs = vol.lock().await.unwrap();
            fs.stall_in(calls);
            fs.fail_truncate();
            drop(fs);
            let pending = poll_once(vol.open("/a.txt", OpenOptions::new().write().truncate()));
            let mut fs = vol.lock().await.unwrap();
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
        vol.lock().await.unwrap().stall_next();
        cancel(vol.metadata("/etc/conf"));
        vol.lock().await.unwrap().stall_next();
        cancel(vol.read_dir("/etc"));
        vol.lock().await.unwrap().stall_next();
        cancel(vol.remove_file("/etc/conf"));
        vol.lock().await.unwrap().stall_next();
        cancel(vol.set_attr("/a.txt", &SetAttr::new()));
        vol.lock().await.unwrap().stall_next();
        cancel(vol.create_dir_all("/x/y"));
        vol.lock().await.unwrap().stall_next();
        cancel(vol.open("/a.txt", OpenOptions::new().write().truncate()));
        vol.remove_file("/a.txt").await.unwrap();
        assert_eq!(vol.lock().await.unwrap().open_nodes(), 1);
        vol.lock().await.unwrap().stall_next();
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
        let vol = SendVolume::new(Sendable::new(fixture()));
        for calls in 0.. {
            vol.lock().await.unwrap().stall_in(calls);
            let pending = poll_once(read_tree(&vol, "/"));
            let fs = vol.lock().await.unwrap();
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
        vol.lock().await.unwrap().stall_in(1);
        assert!(poll_once(async {
            let mut reader = ContentReader::open(content).await.unwrap();
            reader.read_at(0, &mut [0u8; 4]).await.unwrap();
        }));
        assert_eq!(vol.lock().await.unwrap().open_files(), 0);
        drop(tree);
        let fs = vol.into_inner().await.ok().unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

/// Generic over the filesystem: `SendFileSystem` proves its futures
/// `Send`.
fn spawn_read<F: hadris_fs::async_::SendFileSystem + 'static>(
    vol: SendVolume<F>,
    path: &'static str,
) -> std::thread::JoinHandle<FsResult<Vec<u8>, F::DeviceError>> {
    std::thread::spawn(move || block_on(async move { read(&vol, path).await }))
}

#[test]
fn generic_code_spawns_across_threads() {
    let vol = SendVolume::new(Sendable::new(fixture()));
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
        let mut guard = vol.lock().await.unwrap();
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

/// Polls `future` once and drops it. Its output, if it finished.
fn poll_capture<F: core::future::Future>(future: F) -> Option<F::Output> {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        core::task::Poll::Ready(out) => Some(out),
        core::task::Poll::Pending => None,
    }
}

#[test]
fn helpers_handles_and_walks() {
    block_on(async {
        let time = hadris_fs::DateTime::from_unix_seconds(1_700_000_000).unwrap();
        let vol = Volume::new(fixture());
        vol.write("/new.txt", b"fresh").await.unwrap();
        assert_eq!(vol.read("/new.txt").await.unwrap(), b"fresh");
        assert!(vol.exists("/etc/conf").await.unwrap());
        assert!(!vol.exists("/missing").await.unwrap());

        let mut walk = vol.walk("/etc").await.unwrap();
        let mut paths = Vec::new();
        let mut conf = None;
        while let Some(entry) = walk.next_entry().await {
            let entry = entry.unwrap();
            if entry.path() == b"/etc/conf" {
                conf = Some(entry.clone());
            }
            paths.push(entry.into_path());
        }
        assert_eq!(paths, [b"/etc/conf".to_vec(), b"/etc/up".to_vec()]);
        drop(walk);
        let conf = conf.unwrap();

        let file = vol
            .open_node(conf.dir(), conf.entry(), OpenOptions::new().read().write())
            .await
            .unwrap();
        assert_eq!(file.write_at(0, b"KEY").await.unwrap(), 3);
        let mut buf = [0u8; 9];
        assert_eq!(file.read_at(0, &mut buf).await.unwrap(), 9);
        assert_eq!(&buf, b"KEY=value");
        file.set_attr(&SetAttr::new().with_modified(time))
            .await
            .unwrap();
        file.close().await.unwrap();

        let dir = vol.read_dir("/").await.unwrap();
        let made = vol
            .open(
                "/made",
                OpenOptions::new()
                    .write()
                    .create()
                    .with_attr(SetAttr::new().with_modified(time)),
            )
            .await
            .unwrap();
        assert_eq!(made.metadata().await.unwrap().modified(), Some(time));
        made.close().await.unwrap();
        let mut dir = dir;
        let mut a = None;
        while let Some(entry) = dir.next_entry().await {
            let entry = entry.unwrap();
            if entry.name().as_bytes() == b"a.txt" {
                a = Some(entry);
            }
        }
        dir.open(&a.unwrap(), OpenOptions::new().read())
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        drop(dir);
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn dropped_helpers_leave_no_pins_or_opens() {
    block_on(async {
        let vol = Volume::new(fixture());
        let root = vol.lock().await.unwrap().root();
        let mut listing = vol.read_dir("/").await.unwrap();
        let mut a = None;
        while let Some(entry) = listing.next_entry().await {
            let entry = entry.unwrap();
            if entry.name().as_bytes() == b"a.txt" {
                a = Some(entry);
            }
        }
        drop(listing);
        let a = a.unwrap();
        let calls: [(&str, &dyn Fn() -> bool); 6] = [
            ("read", &|| poll_once(vol.read("/etc/conf"))),
            ("write", &|| poll_once(vol.write("/etc/conf", b"key=value"))),
            ("exists", &|| poll_once(vol.exists("/etc/conf"))),
            ("walk", &|| poll_once(vol.walk("/etc"))),
            ("open_node", &|| {
                poll_once(vol.open_node(root, &a, OpenOptions::new().write().truncate()))
            }),
            ("open with attrs", &|| {
                poll_once(
                    vol.open(
                        "/made",
                        OpenOptions::new()
                            .write()
                            .create()
                            .with_attr(SetAttr::new()),
                    ),
                )
            }),
        ];
        for (name, call) in calls {
            for stall in 0.. {
                vol.lock().await.unwrap().stall_in(stall);
                let pending = call();
                let mut fs = vol.lock().await.unwrap();
                assert_eq!(
                    (fs.open_nodes(), fs.open_files()),
                    (1, 0),
                    "{name} stalled at call {stall}"
                );
                fs.stall_in(u32::MAX);
                drop(fs);
                if !pending {
                    break;
                }
            }
        }
        assert_eq!(vol.read("/etc/conf").await.unwrap(), b"key=value");

        let file = vol
            .open("/a.txt", OpenOptions::new().read().write())
            .await
            .unwrap();
        vol.lock().await.unwrap().stall_in(0);
        cancel(file.read_at(0, &mut [0u8; 4]));
        vol.lock().await.unwrap().stall_in(0);
        cancel(file.write_at(0, b"x"));
        vol.lock().await.unwrap().stall_next();
        cancel(file.set_attr(&SetAttr::new()));
        file.close().await.unwrap();
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

#[test]
fn a_dropped_walk_step_loses_no_entry() {
    block_on(async {
        let vol = Volume::new(fixture());
        let all = block_on(async {
            let mut walk = vol.walk("/").await.unwrap();
            let mut all = Vec::new();
            while let Some(entry) = walk.next_entry().await {
                all.push(entry.unwrap().into_path());
            }
            all
        });
        assert_eq!(all.len(), 8);
        for stall in 0.. {
            let mut walk = vol.walk("/").await.unwrap();
            vol.lock().await.unwrap().stall_in(stall);
            let mut paths = Vec::new();
            let mut dropped = false;
            loop {
                match poll_capture(walk.next_entry()) {
                    Some(Some(entry)) => paths.push(entry.unwrap().into_path()),
                    Some(None) => break,
                    None => dropped = true,
                }
            }
            assert_eq!(paths, all, "stalled at call {stall}");
            vol.lock().await.unwrap().stall_in(u32::MAX);
            if !dropped {
                break;
            }
        }
        let fs = vol.into_inner().await.unwrap();
        assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    });
}

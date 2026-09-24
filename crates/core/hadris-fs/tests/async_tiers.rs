//! The async mode runs the same jobs as the sync mode, and lets generic code
//! move its futures to other threads.

#![cfg(all(feature = "async", feature = "std"))]

mod common;

use common::block_on;

mod plain {
    use super::*;
    use common::asynch::{MemFs, fixture};
    use hadris_fs::r#async::{DriverExt, PathExt, Posix, Volume, copy_tree};
    use hadris_fs::{ErrorKind, OpenOptions};
    use hadris_io::r#async::{Read as _, Write as _};

    #[test]
    fn raw_and_shared_tiers() {
        block_on(async {
            let mut fs = MemFs::new();
            fs.create_dir_all("/d").await.unwrap();
            fs.write_file("/d/x", b"data").await.unwrap();
            assert_eq!(fs.read_to_vec("/d/x").await.unwrap(), b"data");
            let mut dir = fs.read_dir("/d").await.unwrap();
            let item = dir.next_entry().await.unwrap().unwrap();
            assert_eq!(item.name_str(), Some("x"));
            assert!(dir.next_entry().await.is_none());
            drop(dir);
            assert_eq!(fs.open_nodes(), 1);

            let vol = Volume::new(fixture().with_resolver(Posix::new()));
            let mut file = vol
                .open("/log", OpenOptions::write().create().append())
                .await
                .unwrap();
            file.write_all(b"one ").await.unwrap();
            file.write_all(b"two").await.unwrap();
            file.close().await.unwrap();
            let mut buf = [0u8; 7];
            let mut log = vol.open("/log", OpenOptions::read()).await.unwrap();
            log.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"one two");
            drop(log);
            assert_eq!(vol.read_to_vec("/link/conf").await.unwrap(), b"key=value");
            assert_eq!(
                vol.read_to_vec("/loop").await.unwrap_err().kind(),
                ErrorKind::Symlink
            );
            assert_eq!(vol.into_inner().into_inner().open_nodes(), 1);
        });
    }

    #[test]
    fn copy_tree_between_tiers() {
        block_on(async {
            let mut src = fixture();
            let vol = Volume::new(MemFs::new());
            copy_tree(&mut src, "/etc", &vol, "/copy/etc")
                .await
                .unwrap();
            assert_eq!(
                vol.read_to_vec("/copy/etc/conf").await.unwrap(),
                b"key=value"
            );
            assert!(
                vol.metadata("/copy/etc/up")
                    .await
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            let err = copy_tree(&mut src, "/nope", &vol, "/x").await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotFound);
            assert_eq!(src.open_nodes(), 1);
            assert_eq!(vol.into_inner().open_nodes(), 1);
        });
    }

    #[test]
    fn a_dropped_file_leaves_publishing_to_the_driver() {
        block_on(async {
            let mut fs = fixture();
            let mut file = fs
                .open("/log", OpenOptions::write().create())
                .await
                .unwrap();
            file.write_all(b"entry").await.unwrap();
            drop(file);
            assert_eq!(fs.publishes(), 0);
            let mut file = fs.open("/log", OpenOptions::write()).await.unwrap();
            file.write_all(b"entry").await.unwrap();
            file.close().await.unwrap();
            assert_eq!(fs.publishes(), 1);
            assert_eq!(fs.open_nodes(), 1);
        });
    }

    #[test]
    fn many_handles_dropped_under_the_lock_are_queued() {
        block_on(async {
            let mut fs = fixture();
            for i in 0..40 {
                fs.write_file(&format!("/h{i}"), b"x").await.unwrap();
            }
            let vol = Volume::new(fs);
            let mut files = Vec::new();
            for i in 0..40 {
                files.push(
                    vol.open(&format!("/h{i}"), OpenOptions::read())
                        .await
                        .unwrap(),
                );
            }
            let guard = vol.lock().await;
            drop(files);
            drop(guard);
            assert_eq!(vol.into_inner().open_nodes(), 1);
        });
    }
}

mod send {
    use std::sync::Arc;

    use super::*;
    use common::asynch::fixture;
    use hadris_fs::FsResult;
    use hadris_fs::OpenOptions;
    use hadris_fs::r#async::{File, FileSystem, PathExt, Volume, copy_tree};
    use hadris_io::r#async::Read as _;

    /// Generic over the filesystem, with no `Send` bounds: the mode's
    /// supertraits prove them.
    fn spawn_read<F: FileSystem + 'static>(
        fs: Arc<F>,
        path: &'static str,
    ) -> std::thread::JoinHandle<FsResult<Vec<u8>, F::DeviceError>> {
        std::thread::spawn(move || {
            block_on(async move {
                let mut file = File::open(fs, path, OpenOptions::read()).await?;
                let mut out = vec![0u8; 64];
                let n = file.read(&mut out).await?;
                out.truncate(n);
                Ok(out)
            })
        })
    }

    fn spawn_helpers<F: FileSystem + 'static>(fs: Arc<F>) -> std::thread::JoinHandle<bool> {
        std::thread::spawn(move || {
            block_on(async move {
                fs.create_dir_all("/spawned").await.unwrap();
                fs.write_file("/spawned/x", b"x").await.unwrap();
                fs.exists("/spawned/x").await.unwrap()
            })
        })
    }

    fn spawn_copy<S: FileSystem + 'static, T: FileSystem + 'static>(
        src: Arc<S>,
        dst: Arc<T>,
    ) -> std::thread::JoinHandle<Result<(), hadris_fs::PathError>> {
        std::thread::spawn(move || block_on(copy_tree(src, "/", dst, "/mirror")))
    }

    #[test]
    fn copy_tree_futures_are_send() {
        let src = Arc::new(Volume::new(fixture()));
        let dst = Arc::new(Volume::new(common::asynch::MemFs::new()));
        spawn_copy(Arc::clone(&src), Arc::clone(&dst))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(
            block_on(dst.read_to_vec("/mirror/etc/conf")).unwrap(),
            b"key=value"
        );
        let src = Arc::into_inner(src).unwrap().into_inner();
        let dst = Arc::into_inner(dst).unwrap().into_inner();
        assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
    }

    #[test]
    fn generic_code_spawns_across_threads() {
        let vol = Arc::new(Volume::new(fixture()));
        let a = spawn_read(Arc::clone(&vol), "/a.txt");
        let b = spawn_read(Arc::clone(&vol), "/etc/conf");
        assert!(spawn_helpers(Arc::clone(&vol)).join().unwrap());
        assert_eq!(a.join().unwrap().unwrap(), b"root a");
        assert_eq!(b.join().unwrap().unwrap(), b"key=value");
        let vol = Arc::into_inner(vol).unwrap();
        assert_eq!(vol.into_inner().open_nodes(), 1);
    }

    #[test]
    fn borrowed_handles_can_be_held_across_awaits() {
        let vol = Volume::new(fixture());
        let task = async {
            let mut file = vol.open("/a.txt", OpenOptions::read()).await.unwrap();
            let mut buf = [0u8; 4];
            file.read(&mut buf).await.unwrap();
            buf
        };
        fn assert_send<T: Send>(t: T) -> T {
            t
        }
        assert_eq!(&block_on(assert_send(task)), b"root");
    }
}

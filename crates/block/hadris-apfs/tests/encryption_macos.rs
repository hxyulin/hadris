#![cfg(all(
    target_os = "macos",
    feature = "encryption",
    feature = "read",
    feature = "std",
    any(feature = "sync", feature = "async")
))]

use std::path::PathBuf;
use std::sync::OnceLock;

const PASSWORD: &[u8] = b"hadris-public-fixture-password";

fn fixture() -> &'static PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        if let Some(directory) = std::env::var_os("HADRIS_APFS_ENCRYPTION_FIXTURES") {
            return PathBuf::from(directory).join("encrypted.dmg");
        }
        let directory = std::env::temp_dir().join(format!(
            "hadris-apfs-encryption-tests-{}",
            std::process::id()
        ));
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../scripts/apfs-encryption-fixtures.py");
        let output = std::process::Command::new("python3")
            .arg(script)
            .arg("--output")
            .arg(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Apple fixture creation failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        directory.join("encrypted.dmg")
    })
}

fn device() -> hadris_storage::MemDevice<Vec<u8>> {
    hadris_storage::MemDevice::new(
        std::fs::read(fixture()).unwrap(),
        hadris_storage::BlockSize::new(512).unwrap(),
    )
}

fn multilevel_fixture() -> bool {
    let manifest = std::fs::read(fixture().parent().unwrap().join("manifest.json")).unwrap();
    manifest
        .windows(b"many/file-000.txt".len())
        .any(|bytes| bytes == b"many/file-000.txt")
}

#[cfg(feature = "sync")]
mod sync_cases {
    use super::{PASSWORD, device};
    use hadris_apfs::{VolumeSelector, sync::ApfsFs};
    use hadris_fs::sync::FileSystem;
    use hadris_fs::{MountOptions, Name, Resolve};

    #[test]
    fn native_encrypted_volume_use_cases() {
        let mut fs =
            ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None).unwrap();
        hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
        let root = fs.root();
        let file = fs.lookup(root, Name::new("hello.txt")).unwrap();
        let hardlink = fs.lookup(root, Name::new("hardlink")).unwrap();
        assert_eq!(file, hardlink);
        assert_eq!(fs.stat(file).unwrap().nlink(), 2);
        let mut bytes = [0xa5; 64];
        let count = fs.read(file, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        assert_eq!(fs.read(file, count as u64, &mut bytes).unwrap(), 0);
        assert_eq!(fs.read(file, u64::MAX, &mut bytes).unwrap(), 0);
        let symlink = fs.lookup(root, Name::new("symlink")).unwrap();
        assert_eq!(fs.readlink(symlink, &mut bytes).unwrap(), b"hello.txt");
        assert_eq!(fs.resolve(b"/symlink", Resolve::Follow).unwrap(), file);
        let pattern = fs.resolve(b"/nested/pattern.bin", Resolve::Follow).unwrap();
        for offset in [0, 509, 4090, 65_530, 299_983] {
            let count = fs.read(pattern, offset, &mut bytes).unwrap();
            assert_eq!(count, (300_000 - offset as usize).min(bytes.len()));
            for (index, value) in bytes[..count].iter().enumerate() {
                assert_eq!(*value, ((offset as usize + index) % 251) as u8);
            }
        }
        let sparse = fs.lookup(root, Name::new("sparse.bin")).unwrap();
        assert_eq!(fs.stat(sparse).unwrap().len(), 2 * 1024 * 1024);
        for offset in [
            0,
            4090,
            1024 * 1024 - 3,
            1024 * 1024 + 4,
            2 * 1024 * 1024 - 4,
        ] {
            let count = fs.read(sparse, offset, &mut bytes).unwrap();
            for (index, value) in bytes[..count].iter().enumerate() {
                let pos = offset as usize + index;
                let expected = if pos < 6 {
                    b"begin\n"[pos]
                } else if (1024 * 1024..1024 * 1024 + 4).contains(&pos) {
                    b"end\n"[pos - 1024 * 1024]
                } else {
                    0
                };
                assert_eq!(*value, expected, "sparse byte {pos}");
            }
        }
        let uuid = fs.volume_superblock().volume_id;
        drop(fs);
        let selected = ApfsFs::mount_volume_with_password(
            device(),
            MountOptions::new(),
            VolumeSelector::Uuid(uuid),
            PASSWORD,
            Some(uuid),
        )
        .unwrap();
        assert_eq!(selected.volume_superblock().volume_id, uuid);
    }

    #[test]
    fn encrypted_native_reads_require_exact_volume_scope() {
        let fs =
            ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None).unwrap();
        let volume = fs.volume_superblock().clone();
        let mut container = fs.into_container();
        let entry = container
            .resolve_path(&volume, "/hello.txt")
            .unwrap()
            .unwrap();
        let inode = container
            .inode_record(&volume, entry.file_id)
            .unwrap()
            .unwrap();
        let extents = container.file_extents(&volume, inode.private_id).unwrap();
        assert!(extents.iter().any(|extent| extent.cryptography_id != 0));
        let mut bytes = [0xa5; 64];
        assert!(
            container
                .read_extents_at(&extents, 30, 0, &mut bytes)
                .is_err()
        );
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        let mut foreign = volume.clone();
        foreign.volume_id[0] ^= 1;
        let err = container
            .read_volume_extents_at(&foreign, &extents, 30, 0, &mut bytes)
            .unwrap_err();
        assert_eq!(
            hadris_apfs::Detail::of(&err),
            Some(hadris_apfs::Detail::Credentials)
        );
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        let count = container
            .read_volume_extents_at(&volume, &extents, 30, 0, &mut bytes)
            .unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        let root = container
            .resolve_volume_object(&volume, volume.root_tree_oid)
            .unwrap()
            .unwrap();
        assert!(
            container
                .read_btree_node_with_flags(root.address, root.flags)
                .is_err()
        );
        assert!(
            container
                .read_volume_btree_node_with_flags(&foreign, root.address, root.flags)
                .is_err()
        );
        let node = container
            .read_volume_btree_node_with_flags(&volume, root.address, root.flags)
            .unwrap();
        if super::multilevel_fixture() {
            assert!(node.node().unwrap().level > 0);
        }
    }

    #[test]
    fn native_unlock_retry_and_driver_key_invalidation() {
        let mut fs =
            ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None).unwrap();
        let root = fs.root();
        let file = fs.lookup(root, Name::new("hello.txt")).unwrap();
        let volume = fs.volume_superblock().clone();
        let wrong = fs
            .container_mut()
            .unlock_volume(&volume, b"wrong-password", None)
            .unwrap_err();
        assert_eq!(
            hadris_apfs::Detail::of(&wrong),
            Some(hadris_apfs::Detail::Credentials)
        );
        let mut bytes = [0xa5; 64];
        assert!(fs.read(file, 0, &mut bytes).is_err());
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        fs.container_mut()
            .unlock_volume(&volume, PASSWORD, Some(volume.volume_id))
            .unwrap();
        let count = fs.read(file, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        let mut container = fs.into_container();
        let entry = container
            .resolve_path(&volume, "/hello.txt")
            .unwrap()
            .unwrap();
        assert_eq!(
            container.read_file(&volume, entry.file_id, 64).unwrap(),
            b"hadris encrypted APFS fixture\n"
        );
        assert!(
            container
                .unlock_volume(&volume, b"wrong-password", None)
                .is_err()
        );
        assert!(container.resolve_path(&volume, "/hello.txt").is_err());
        container.unlock_volume(&volume, PASSWORD, None).unwrap();
        assert_eq!(
            container.read_file(&volume, entry.file_id, 64).unwrap(),
            b"hadris encrypted APFS fixture\n"
        );
    }

    #[test]
    fn rejected_credentials_retain_device_and_never_print_secrets() {
        let expected = std::fs::read(super::fixture()).unwrap();
        let absent = ApfsFs::mount(device(), MountOptions::new()).err().unwrap();
        assert_eq!(absent.into_device().into_inner(), expected);
        for (password, crypto_user) in [
            (&b"wrong-native-fixture-password"[..], None),
            (PASSWORD, Some([0xff; 16])),
        ] {
            let err =
                ApfsFs::mount_with_password(device(), MountOptions::new(), password, crypto_user)
                    .err()
                    .unwrap();
            assert_eq!(err.kind(), hadris_fs::ErrorKind::InvalidInput);
            assert_eq!(
                hadris_apfs::Detail::of(err.error()),
                Some(hadris_apfs::Detail::Credentials)
            );
            for diagnostic in [format!("{err}"), format!("{err:?}")] {
                assert!(!diagnostic.contains(std::str::from_utf8(password).unwrap()));
            }
            assert_eq!(err.into_device().into_inner(), expected);
        }
    }
}

#[cfg(feature = "async")]
mod async_cases {
    use super::{PASSWORD, device};
    use hadris_apfs::{VolumeSelector, r#async::ApfsFs};
    use hadris_fs::r#async::FileSystem;
    use hadris_fs::{MountOptions, Name, Resolve};

    async fn native_encrypted_volume_use_cases() {
        let mut fs = ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None)
            .await
            .unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut fs)
            .await
            .unwrap();
        let root = fs.root();
        let file = fs.lookup(root, Name::new("hello.txt")).await.unwrap();
        let hardlink = fs.lookup(root, Name::new("hardlink")).await.unwrap();
        assert_eq!(file, hardlink);
        assert_eq!(fs.stat(file).await.unwrap().nlink(), 2);
        let mut bytes = [0xa5; 64];
        let count = fs.read(file, 0, &mut bytes).await.unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        assert_eq!(fs.read(file, count as u64, &mut bytes).await.unwrap(), 0);
        assert_eq!(fs.read(file, u64::MAX, &mut bytes).await.unwrap(), 0);
        let symlink = fs.lookup(root, Name::new("symlink")).await.unwrap();
        assert_eq!(
            fs.readlink(symlink, &mut bytes).await.unwrap(),
            b"hello.txt"
        );
        assert_eq!(
            fs.resolve(b"/symlink", Resolve::Follow).await.unwrap(),
            file
        );
        let pattern = fs
            .resolve(b"/nested/pattern.bin", Resolve::Follow)
            .await
            .unwrap();
        for offset in [0, 509, 4090, 65_530, 299_983] {
            let count = fs.read(pattern, offset, &mut bytes).await.unwrap();
            assert_eq!(count, (300_000 - offset as usize).min(bytes.len()));
            for (index, value) in bytes[..count].iter().enumerate() {
                assert_eq!(*value, ((offset as usize + index) % 251) as u8);
            }
        }
        let sparse = fs.lookup(root, Name::new("sparse.bin")).await.unwrap();
        assert_eq!(fs.stat(sparse).await.unwrap().len(), 2 * 1024 * 1024);
        for offset in [
            0,
            4090,
            1024 * 1024 - 3,
            1024 * 1024 + 4,
            2 * 1024 * 1024 - 4,
        ] {
            let count = fs.read(sparse, offset, &mut bytes).await.unwrap();
            for (index, value) in bytes[..count].iter().enumerate() {
                let pos = offset as usize + index;
                let expected = if pos < 6 {
                    b"begin\n"[pos]
                } else if (1024 * 1024..1024 * 1024 + 4).contains(&pos) {
                    b"end\n"[pos - 1024 * 1024]
                } else {
                    0
                };
                assert_eq!(*value, expected, "sparse byte {pos}");
            }
        }
        let uuid = fs.volume_superblock().volume_id;
        drop(fs);
        let selected = ApfsFs::mount_volume_with_password(
            device(),
            MountOptions::new(),
            VolumeSelector::Uuid(uuid),
            PASSWORD,
            Some(uuid),
        )
        .await
        .unwrap();
        assert_eq!(selected.volume_superblock().volume_id, uuid);
    }

    async fn encrypted_native_reads_require_exact_volume_scope() {
        let fs = ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None)
            .await
            .unwrap();
        let volume = fs.volume_superblock().clone();
        let mut container = fs.into_container();
        let entry = container
            .resolve_path(&volume, "/hello.txt")
            .await
            .unwrap()
            .unwrap();
        let inode = container
            .inode_record(&volume, entry.file_id)
            .await
            .unwrap()
            .unwrap();
        let extents = container
            .file_extents(&volume, inode.private_id)
            .await
            .unwrap();
        assert!(extents.iter().any(|extent| extent.cryptography_id != 0));
        let mut bytes = [0xa5; 64];
        assert!(
            container
                .read_extents_at(&extents, 30, 0, &mut bytes)
                .await
                .is_err()
        );
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        let mut foreign = volume.clone();
        foreign.volume_id[0] ^= 1;
        let err = container
            .read_volume_extents_at(&foreign, &extents, 30, 0, &mut bytes)
            .await
            .unwrap_err();
        assert_eq!(
            hadris_apfs::Detail::of(&err),
            Some(hadris_apfs::Detail::Credentials)
        );
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        let count = container
            .read_volume_extents_at(&volume, &extents, 30, 0, &mut bytes)
            .await
            .unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        let root = container
            .resolve_volume_object(&volume, volume.root_tree_oid)
            .await
            .unwrap()
            .unwrap();
        assert!(
            container
                .read_btree_node_with_flags(root.address, root.flags)
                .await
                .is_err()
        );
        assert!(
            container
                .read_volume_btree_node_with_flags(&foreign, root.address, root.flags)
                .await
                .is_err()
        );
        let node = container
            .read_volume_btree_node_with_flags(&volume, root.address, root.flags)
            .await
            .unwrap();
        if super::multilevel_fixture() {
            assert!(node.node().unwrap().level > 0);
        }
    }

    async fn native_unlock_retry_and_driver_key_invalidation() {
        let mut fs = ApfsFs::mount_with_password(device(), MountOptions::new(), PASSWORD, None)
            .await
            .unwrap();
        let root = fs.root();
        let file = fs.lookup(root, Name::new("hello.txt")).await.unwrap();
        let volume = fs.volume_superblock().clone();
        let wrong = fs
            .container_mut()
            .unlock_volume(&volume, b"wrong-password", None)
            .await
            .unwrap_err();
        assert_eq!(
            hadris_apfs::Detail::of(&wrong),
            Some(hadris_apfs::Detail::Credentials)
        );
        let mut bytes = [0xa5; 64];
        assert!(fs.read(file, 0, &mut bytes).await.is_err());
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
        fs.container_mut()
            .unlock_volume(&volume, PASSWORD, Some(volume.volume_id))
            .await
            .unwrap();
        let count = fs.read(file, 0, &mut bytes).await.unwrap();
        assert_eq!(&bytes[..count], b"hadris encrypted APFS fixture\n");
        let mut container = fs.into_container();
        let entry = container
            .resolve_path(&volume, "/hello.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            container
                .read_file(&volume, entry.file_id, 64)
                .await
                .unwrap(),
            b"hadris encrypted APFS fixture\n"
        );
        assert!(
            container
                .unlock_volume(&volume, b"wrong-password", None)
                .await
                .is_err()
        );
        assert!(container.resolve_path(&volume, "/hello.txt").await.is_err());
        container
            .unlock_volume(&volume, PASSWORD, None)
            .await
            .unwrap();
        assert_eq!(
            container
                .read_file(&volume, entry.file_id, 64)
                .await
                .unwrap(),
            b"hadris encrypted APFS fixture\n"
        );
    }

    async fn rejected_credentials_retain_device_and_never_print_secrets() {
        let expected = std::fs::read(super::fixture()).unwrap();
        let absent = ApfsFs::mount(device(), MountOptions::new())
            .await
            .err()
            .unwrap();
        assert_eq!(absent.into_device().into_inner(), expected);
        for (password, crypto_user) in [
            (&b"wrong-native-fixture-password"[..], None),
            (PASSWORD, Some([0xff; 16])),
        ] {
            let err =
                ApfsFs::mount_with_password(device(), MountOptions::new(), password, crypto_user)
                    .await
                    .err()
                    .unwrap();
            assert_eq!(err.kind(), hadris_fs::ErrorKind::InvalidInput);
            assert_eq!(
                hadris_apfs::Detail::of(err.error()),
                Some(hadris_apfs::Detail::Credentials)
            );
            for diagnostic in [format!("{err}"), format!("{err:?}")] {
                assert!(!diagnostic.contains(std::str::from_utf8(password).unwrap()));
            }
            assert_eq!(err.into_device().into_inner(), expected);
        }
    }

    #[test]
    fn async_native_encrypted_volume_use_cases() {
        block_on(native_encrypted_volume_use_cases());
    }
    #[test]
    fn async_rejected_credentials_retain_device_and_never_print_secrets() {
        block_on(rejected_credentials_retain_device_and_never_print_secrets());
    }
    #[test]
    fn async_native_unlock_retry_and_driver_key_invalidation() {
        block_on(native_unlock_retry_and_driver_key_invalidation());
    }
    #[test]
    fn async_encrypted_native_reads_require_exact_volume_scope() {
        block_on(encrypted_native_reads_require_exact_volume_scope());
    }

    struct GatedDevice {
        inner: hadris_storage::MemDevice<Vec<u8>>,
        stop_at: std::sync::Arc<std::sync::atomic::AtomicU64>,
        reached: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }
    impl hadris_io::ErrorType for GatedDevice {
        type Error = core::convert::Infallible;
    }
    impl hadris_storage::r#async::BlockDevice for GatedDevice {
        fn block_size(&self) -> hadris_storage::BlockSize {
            hadris_storage::r#async::BlockDevice::block_size(&self.inner)
        }
        fn block_count(&self) -> u64 {
            hadris_storage::r#async::BlockDevice::block_count(&self.inner)
        }
        async fn read_blocks(
            &mut self,
            start: hadris_storage::BlockIndex,
            bytes: &mut [u8],
        ) -> Result<(), hadris_io::Error<Self::Error>> {
            use std::sync::atomic::Ordering;
            if start.get() == self.stop_at.load(Ordering::SeqCst) {
                self.reached.store(true, Ordering::SeqCst);
                std::future::pending::<()>().await;
            }
            hadris_storage::r#async::BlockDevice::read_blocks(&mut self.inner, start, bytes).await
        }
    }

    #[test]
    fn canceled_unlock_does_not_install_candidate_or_preserve_previous_key() {
        use std::future::Future;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        };
        use std::task::{Context, Poll, Waker};
        let stop_at = Arc::new(AtomicU64::new(u64::MAX));
        let reached = Arc::new(AtomicBool::new(false));
        let gated = GatedDevice {
            inner: device(),
            stop_at: stop_at.clone(),
            reached: reached.clone(),
        };
        let mut container = block_on(hadris_apfs::r#async::Container::open(gated)).unwrap();
        let checkpoint = block_on(container.latest_superblock()).unwrap();
        let volume = block_on(container.volume_superblocks(&checkpoint))
            .unwrap()
            .remove(0);
        block_on(container.unlock_volume(&volume, PASSWORD, None)).unwrap();
        let root = block_on(container.resolve_volume_object(&volume, volume.root_tree_oid))
            .unwrap()
            .unwrap();
        let sectors = u64::from(checkpoint.block_size / 512);
        stop_at.store(root.address * sectors, Ordering::SeqCst);
        let mut unlock = Box::pin(container.unlock_volume(&volume, PASSWORD, None));
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(unlock.as_mut().poll(&mut context), Poll::Pending));
        assert!(reached.load(Ordering::SeqCst));
        drop(unlock);
        stop_at.store(u64::MAX, Ordering::SeqCst);
        assert!(block_on(container.resolve_path(&volume, "/hello.txt")).is_err());
        let refused = block_on(container.read_volume_btree_node_with_flags(
            &volume,
            root.address,
            root.flags,
        ))
        .unwrap_err();
        assert_eq!(
            hadris_apfs::Detail::of(&refused),
            Some(hadris_apfs::Detail::Feature)
        );
        block_on(container.unlock_volume(&volume, PASSWORD, None)).unwrap();
        assert!(
            block_on(container.resolve_path(&volume, "/hello.txt"))
                .unwrap()
                .is_some()
        );
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};
        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::park(),
            }
        }
    }
}

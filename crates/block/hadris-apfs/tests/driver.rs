#![cfg(all(feature = "read", feature = "sync", feature = "alloc"))]

mod common;

use common::*;
use hadris_apfs::{VolumeSelector, sync::ApfsFs};
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, FileType, MountOptions, Name, NodeId, OpenMode, SetAttr};
use hadris_storage::{BlockSize, MemDevice};

fn device(image: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(image, BlockSize::new(BLOCK as u32).unwrap())
}

fn mount(image: Vec<u8>) -> ApfsFs<MemDevice<Vec<u8>>> {
    ApfsFs::mount(device(image), MountOptions::new()).unwrap()
}

#[test]
fn generic_driver_contract_and_sealed_tree() {
    for image in [build_image(), build_image_variant(Variant::Sealed)] {
        let mut fs = mount(image);
        hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    }
}

#[test]
fn vfs_cursors_ids_open_pins_and_random_reads() {
    let mut fs = mount(build_image());
    let root = fs.root();
    let mut cursor = DirCursor::START;
    let mut entries = Vec::new();
    while let Some(entry) = fs.readdir(root, cursor).unwrap() {
        cursor = entry.next_cursor();
        entries.push((entry.name().as_bytes().to_vec(), entry.node(), cursor));
    }
    assert_eq!(entries.len() as u64, FILE_COUNT);
    assert!(fs.readdir(root, cursor).unwrap().is_none());
    assert!(fs.readdir(root, cursor).unwrap().is_none());
    assert_eq!(
        fs.readdir(root, DirCursor::from_raw(u64::MAX))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    for (name, listed_id, resume) in &entries {
        let id = fs.lookup(root, Name::new(name)).unwrap();
        assert_eq!(id, *listed_id);
        let meta = fs.stat(id).unwrap();
        assert_eq!(meta.file_type(), FileType::File);
        assert_eq!(meta.nlink(), 1);
        fs.open(id, OpenMode::Read).unwrap();
        fs.forget(id, u64::MAX);
        let mut head = [0; 4];
        assert_eq!(fs.read(id, 4, &mut head).unwrap(), 4);
        assert_eq!(&head, b"ents");
        assert_eq!(fs.read(id, meta.len(), &mut head).unwrap(), 0);
        assert_eq!(fs.read(id, u64::MAX, &mut head).unwrap(), 0);
        fs.close(id).unwrap();
        let resumed = fs.readdir(root, *resume).unwrap();
        if let Some(resumed) = resumed {
            assert_ne!(resumed.node(), id);
        }
    }
    assert_eq!(fs.parent(root).unwrap(), root);
    let stats = fs.statfs().unwrap();
    assert_eq!(stats.total_blocks(), IMAGE_BLOCKS as u64);
    assert_eq!(stats.block_size(), BLOCK as u32);
    assert!(stats.free_blocks() <= stats.total_blocks());
    let forged = NodeId::new(u64::MAX).unwrap();
    assert_eq!(
        fs.stat(forged).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
    assert_eq!(
        fs.lookup(root, Name::new("missing")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert_eq!(
        fs.create(root, Name::new("new"), &SetAttr::new())
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
}

#[test]
fn hardlinks_symlinks_and_sparse_files_share_v3_semantics() {
    let mut fs = mount(use_case_image());
    let root = fs.root();
    let original = fs.lookup(root, Name::new("file0.txt")).unwrap();
    let alias = fs.lookup(root, Name::new("alias.txt")).unwrap();
    assert_eq!(alias, original);
    let meta = fs.stat(alias).unwrap();
    assert_eq!(meta.nlink(), 2);
    assert_eq!(meta.permissions().bits(), 0o644);
    assert_eq!(meta.owner(), Some(hadris_fs::Owner::new(501, 20)));
    assert_eq!(meta.created().unwrap().unix_seconds(), 1_577_836_800);
    assert_eq!(meta.modified().unwrap().unix_seconds(), 1_577_836_801);
    assert_eq!(meta.changed().unwrap().unix_seconds(), 1_577_836_802);
    assert_eq!(meta.accessed().unwrap().unix_seconds(), 1_577_836_803);
    let link = fs.lookup(root, Name::new("link")).unwrap();
    assert_eq!(fs.stat(link).unwrap().file_type(), FileType::Symlink);
    let mut target = [0; 64];
    assert_eq!(fs.readlink(link, &mut target).unwrap(), b"file0.txt");
    let sparse = fs.lookup(root, Name::new("sparse.txt")).unwrap();
    let expected = holey_contents(0);
    let mut out = vec![0xff; expected.len() + 64];
    let n = fs.read(sparse, 0, &mut out).unwrap();
    assert_eq!(&out[..n], expected);
    let n = fs
        .read(sparse, (2 * BLOCK - 4) as u64, &mut target[..10])
        .unwrap();
    assert_eq!(&target[..n], &expected[2 * BLOCK - 4..2 * BLOCK + 6]);
    assert_eq!(
        fs.open(alias, OpenMode::Write).unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.write(alias, 0, b"new").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.truncate(alias, 0).unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
}

#[test]
fn multi_volume_mount_requires_an_explicit_selection_and_returns_the_device() {
    let image = multi_volume_image();
    let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()) else {
        panic!("an ambiguous container mounted");
    };
    assert_eq!(error.into_device().into_inner(), image);
    for selector in [
        VolumeSelector::Index(1),
        VolumeSelector::ObjectId(2048),
        VolumeSelector::Uuid([1; 16]),
        VolumeSelector::Name("Other"),
    ] {
        let mut fs =
            ApfsFs::mount_volume(device(image.clone()), MountOptions::new(), selector).unwrap();
        let root = fs.root();
        assert!(fs.lookup(root, Name::new("file5.txt")).is_ok());
    }
    let Err(error) = ApfsFs::mount_volume(
        device(image.clone()),
        MountOptions::new(),
        VolumeSelector::Index(2),
    ) else {
        panic!("a missing volume mounted");
    };
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(error.into_device().into_inner(), image);
}

#[cfg(feature = "std")]
#[test]
fn volume_handles_list_read_seek_follow_links_and_extract() {
    use hadris_fs::sync::{Volume, read_tree};
    use hadris_fs::{OpenOptions, Resolve};
    use hadris_io::SeekFrom;

    let vol = Volume::with_resolve(mount(use_case_image()), Resolve::Follow);
    let mut dir = vol.read_dir("/").unwrap();
    let mut names = Vec::new();
    while let Some(entry) = dir.next_entry() {
        names.push(entry.unwrap().name().as_bytes().to_vec());
    }
    assert!(names.iter().any(|name| name == b"alias.txt"));
    let mut file = vol.open("/link", OpenOptions::new().read()).unwrap();
    file.seek(SeekFrom::Start(4)).unwrap();
    let mut buf = [0; 4];
    assert_eq!(file.read(&mut buf).unwrap(), 4);
    assert_eq!(&buf, b"ents");
    file.close().unwrap();
    assert_eq!(vol.read_link("/link").unwrap(), b"file0.txt");
    assert_eq!(
        vol.symlink_metadata("/link").unwrap().file_type(),
        FileType::Symlink
    );
    let tree = read_tree(&vol, "/").unwrap();
    assert!(tree.get("file0.txt").is_some());
    assert_eq!(tree.get("link").unwrap().target(), Some(&b"file0.txt"[..]));
    assert_eq!(
        vol.create_dir("/new").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
}

#[test]
fn duplicate_volume_names_do_not_pick_the_first_match() {
    let image = duplicate_volume_names_image();
    let Err(error) = ApfsFs::mount_volume(
        device(image.clone()),
        MountOptions::new(),
        VolumeSelector::Name("Test"),
    ) else {
        panic!("duplicate volume names silently mounted");
    };
    assert_eq!(error.into_device().into_inner(), image);
}

#[test]
fn huge_logical_files_allow_bounded_random_access() {
    let mut fs = mount(build_image_variant(Variant::HugeFile));
    let root = fs.root();
    let node = fs.lookup(root, Name::new("file0.txt")).unwrap();
    assert_eq!(fs.stat(node).unwrap().len(), u64::MAX);
    let mut buf = [0xff; 8];
    assert_eq!(fs.read(node, 0, &mut buf).unwrap(), 8);
    assert_eq!(&buf, &file_contents(0)[..8]);
    assert_eq!(fs.read(node, 1 << 50, &mut buf).unwrap(), 8);
    assert_eq!(buf, [0; 8]);
}

#[test]
fn unidentified_images_and_bad_checksums_have_distinct_errors() {
    let Err(error) = ApfsFs::mount(device(vec![0; BLOCK * IMAGE_BLOCKS]), MountOptions::new())
    else {
        panic!("an empty image mounted");
    };
    assert_eq!(error.kind(), ErrorKind::NotRecognized);
    for mut image in [
        build_image(),
        checkpoint_image(&[(21, 4, None, false), (22, 6, None, false)]),
    ] {
        image[500] ^= 1;
        let Err(error) = ApfsFs::mount(device(image), MountOptions::new()) else {
            panic!("a corrupt container mounted");
        };
        assert_eq!(error.kind(), ErrorKind::Corrupt);
        assert_eq!(
            hadris_apfs::Detail::of(error.error()),
            Some(hadris_apfs::Detail::Checksum)
        );
    }
}

#[test]
fn a_bad_block_zero_checksum_mounts_from_a_valid_checkpoint() {
    let mut image = checkpoint_image(&[(21, 4, None, true), (22, 6, None, false)]);
    image[500] ^= 1;
    image[72] ^= 1;
    let mut container = hadris_apfs::sync::Container::open(device(image.clone())).unwrap();
    let latest = container.latest_superblock().unwrap();
    assert_eq!(latest.object.transaction_identifier, 4);
    assert_eq!(container.superblock(), &latest);
    let mut fs = mount(image);
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    assert_eq!(fs.checkpoint().object.transaction_identifier, 4);
}

#[test]
fn block_zero_geometry_is_checked_without_its_checksum() {
    let mut image = checkpoint_image(&[(21, 4, None, true)]);
    image[112..120].copy_from_slice(&(IMAGE_BLOCKS as u64 - 1).to_le_bytes());
    let checksum = hadris_apfs::types::checksum::fletcher64(&image[..BLOCK]).unwrap();
    image[..8].copy_from_slice(&checksum.to_le_bytes());
    let Err(error) = hadris_apfs::sync::Container::open(device(image)) else {
        panic!("a descriptor area outside the container opened");
    };
    assert_eq!(error.kind(), ErrorKind::Corrupt);
}

fn corrupt_newest_checkpoint_image(map: bool) -> Vec<u8> {
    use hadris_apfs::types::checksum::fletcher64;
    let mut image = checkpoint_image(&[(21, 4, None, true)]);
    let mut newest = image[..BLOCK].to_vec();
    newest[16..24].copy_from_slice(&6_u64.to_le_bytes());
    newest[136..140].copy_from_slice(&1_u32.to_le_bytes());
    newest[140..144].copy_from_slice(&2_u32.to_le_bytes());
    if !map {
        newest[160..168].copy_from_slice(&19_u64.to_le_bytes());
    }
    let checksum = fletcher64(&newest).unwrap();
    newest[..8].copy_from_slice(&checksum.to_le_bytes());
    image[23 * BLOCK..24 * BLOCK].copy_from_slice(&newest);
    let mut checkpoint_map = vec![0_u8; BLOCK];
    checkpoint_map[8..16].copy_from_slice(&22_u64.to_le_bytes());
    checkpoint_map[16..24].copy_from_slice(&6_u64.to_le_bytes());
    checkpoint_map[24..28].copy_from_slice(&(0x4000_000c_u32).to_le_bytes());
    checkpoint_map[32..36].copy_from_slice(&1_u32.to_le_bytes());
    let checksum = fletcher64(&checkpoint_map).unwrap();
    checkpoint_map[..8].copy_from_slice(&checksum.to_le_bytes());
    if map {
        checkpoint_map[100] ^= 1;
    }
    image[22 * BLOCK..23 * BLOCK].copy_from_slice(&checkpoint_map);
    image
}

#[test]
fn an_unverifiable_newest_checkpoint_falls_back_to_an_older_one() {
    for map in [true, false] {
        let image = corrupt_newest_checkpoint_image(map);
        assert_eq!(latest_xid(image.clone()), 4, "map corrupt: {map}");
        let mut fs = mount(image);
        hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
        assert_eq!(fs.checkpoint().object.transaction_identifier, 4);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceFailure(u32);
impl core::fmt::Display for DeviceFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "device failure {}", self.0)
    }
}
impl core::error::Error for DeviceFailure {}

struct ObservedDevice {
    inner: MemDevice<Vec<u8>>,
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl hadris_io::ErrorType for ObservedDevice {
    type Error = DeviceFailure;
}
impl hadris_storage::sync::BlockDevice for ObservedDevice {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn writable(&self) -> bool {
        true
    }
    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        use std::sync::atomic::Ordering::SeqCst;
        self.reads.fetch_add(1, SeqCst);
        if self.fail.load(SeqCst) {
            return Err(hadris_io::Error::device(
                DeviceFailure(73),
                "test read failed",
            ));
        }
        self.inner
            .read_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }
    fn write_blocks(
        &mut self,
        _: hadris_storage::BlockIndex,
        _: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        panic!("read-only APFS attempted a device write")
    }
}

#[test]
fn backend_payloads_survive_mount_and_read_failures_and_mutations_never_touch_storage() {
    use hadris_storage::sync::BlockDevice;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    };

    let reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(true));
    let make = || ObservedDevice {
        inner: device(build_image()),
        reads: reads.clone(),
        writes: writes.clone(),
        fail: fail.clone(),
    };
    let Err(error) = ApfsFs::mount(make(), MountOptions::new()) else {
        panic!("failing storage mounted");
    };
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(error.error().device_error(), Some(&DeviceFailure(73)));
    assert_eq!(error.into_device().block_count(), IMAGE_BLOCKS as u64);
    fail.store(false, SeqCst);
    let mut fs = ApfsFs::mount(make(), MountOptions::new()).unwrap();
    let root = fs.root();
    let node = fs.lookup(root, Name::new("file0.txt")).unwrap();
    let before = reads.load(SeqCst);
    assert_eq!(
        fs.open(node, OpenMode::Write).unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.write(node, 0, b"changed").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.truncate(node, 0).unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fs.create(root, Name::new("new"), &SetAttr::new())
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(reads.load(SeqCst), before);
    assert_eq!(writes.load(SeqCst), 0);
    fail.store(true, SeqCst);
    let error = fs.read(node, 0, &mut [0; 8]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(error.device_error(), Some(&DeviceFailure(73)));
}

#[test]
fn mount_refuses_dangling_entries_wrong_types_duplicate_names_and_outside_extents() {
    for case in ["dangling", "type", "duplicate", "extent"] {
        let image = invalid_directory_image(case);
        let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()) else {
            panic!("malformed {case} mounted");
        };
        assert_eq!(error.kind(), ErrorKind::Corrupt, "{case}");
        assert_eq!(error.into_device().into_inner(), image);
    }
}

#[test]
fn invalid_utf8_directory_names_are_corrupt_and_return_the_device() {
    let image = invalid_directory_image("utf8");
    let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()) else {
        panic!("invalid UTF-8 directory name mounted");
    };
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert_eq!(error.into_device().into_inner(), image);
}

fn checkpoint_image(copies: &[(usize, u64, Option<u8>, bool)]) -> Vec<u8> {
    use hadris_apfs::types::checksum::fletcher64;
    let mut image = build_image();
    let mut zero = image[..BLOCK].to_vec();
    zero[104..108].copy_from_slice(&3_u32.to_le_bytes());
    zero[112..120].copy_from_slice(&21_u64.to_le_bytes());
    zero[136..140].copy_from_slice(&2_u32.to_le_bytes());
    zero[140..144].copy_from_slice(&1_u32.to_le_bytes());
    let seal = |block: &mut Vec<u8>| {
        let checksum = fletcher64(block).unwrap();
        block[..8].copy_from_slice(&checksum.to_le_bytes());
    };
    seal(&mut zero);
    image[..BLOCK].copy_from_slice(&zero);
    for &(index, xid, uuid, valid) in copies {
        let mut copy = zero.clone();
        copy[16..24].copy_from_slice(&xid.to_le_bytes());
        if let Some(byte) = uuid {
            copy[72] = byte;
        }
        seal(&mut copy);
        if !valid {
            copy[0] ^= 1;
        }
        image[index * BLOCK..(index + 1) * BLOCK].copy_from_slice(&copy);
    }
    image
}

fn latest_xid(image: Vec<u8>) -> u64 {
    let mut container = hadris_apfs::sync::Container::open(device(image)).unwrap();
    container
        .latest_superblock()
        .unwrap()
        .object
        .transaction_identifier
}

#[test]
fn mount_takes_the_newest_valid_checkpoint_in_the_whole_descriptor_area() {
    let image = checkpoint_image(&[
        (23, 2, None, true),
        (21, 4, None, true),
        (22, 6, None, false),
    ]);
    assert_eq!(latest_xid(image.clone()), 4);
    let mut fs = mount(image);
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();

    let image = checkpoint_image(&[
        (23, 2, None, true),
        (21, 4, None, true),
        (22, 6, Some(0xAA), true),
    ]);
    assert_eq!(latest_xid(image), 4);

    let image = checkpoint_image(&[(23, 2, None, false), (22, 6, Some(0xAA), true)]);
    assert_eq!(latest_xid(image), 1);
}

fn reseal(image: &mut [u8], block: usize, offset: usize, value: u64) {
    let block = &mut image[block * BLOCK..(block + 1) * BLOCK];
    block[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    let checksum = hadris_apfs::types::checksum::fletcher64(block).unwrap();
    block[..8].copy_from_slice(&checksum.to_le_bytes());
}

fn read_all_files(fs: &mut ApfsFs<MemDevice<Vec<u8>>>) {
    let root = fs.root();
    for index in 0..FILE_COUNT {
        let node = fs.lookup(root, Name::new(&file_name(index))).unwrap();
        let mut buf = vec![0; BLOCK];
        let n = fs.read(node, 0, &mut buf).unwrap();
        assert_eq!(&buf[..n], file_contents(index));
    }
}

#[test]
fn directory_keys_follow_the_volume_name_features() {
    let mut fs = mount(build_image_variant(Variant::PlainKeys));
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    read_all_files(&mut fs);
    for flags in [1, 8, 9] {
        let mut image = build_image();
        set_volume_incompatible_features(&mut image, flags);
        read_all_files(&mut mount(image));
    }
}

#[test]
fn sealed_volumes_read_extents_from_the_file_extent_tree() {
    let mut fs = mount(build_image_variant(Variant::SealedFext));
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    read_all_files(&mut fs);
}

#[test]
fn unreadable_incompatible_features_refuse_the_mount() {
    for (block, offset, flags) in [
        (0, 64, 0x1),
        (0, 64, 0x100),
        (0, 64, 0x200),
        (3, 56, DEFAULT_VOLUME_INCOMPAT | 0x10),
        (3, 56, DEFAULT_VOLUME_INCOMPAT | 0x40),
        (3, 56, DEFAULT_VOLUME_INCOMPAT | 0x80),
        (3, 56, DEFAULT_VOLUME_INCOMPAT | (1 << 63)),
    ] {
        let mut image = build_image();
        reseal(&mut image, block, offset, flags);
        let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()) else {
            panic!("incompatible feature {flags:#x} mounted");
        };
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{flags:#x}");
        assert_eq!(
            hadris_apfs::Detail::of(error.error()),
            Some(hadris_apfs::Detail::Feature)
        );
        assert_eq!(error.into_device().into_inner(), image);
    }
    for (block, offset, flags) in [(0, 64, 0x2), (3, 56, DEFAULT_VOLUME_INCOMPAT | 0x2 | 0x4)] {
        let mut image = build_image();
        reseal(&mut image, block, offset, flags);
        read_all_files(&mut mount(image));
    }
}

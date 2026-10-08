#[path = "../benches/support/mod.rs"]
mod support;

use std::cell::Cell;
use std::convert::Infallible;

use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};
use support::{Counted, IoCounts, WriteBreakdown};

#[test]
fn counts_calls_and_bytes_separately_and_can_reset_after_setup() {
    let counts = Cell::new(IoCounts::default());
    let mut dev = Counted {
        inner: MemDevice::new(vec![0; 2048], BlockSize::new(512).unwrap()),
        counts: &counts,
        written_blocks: None,
    };
    dev.write_blocks(BlockIndex::new(0), &[7; 1024]).unwrap();
    dev.write_blocks(BlockIndex::new(2), &[9; 512]).unwrap();
    let mut buf = [0; 1536];
    dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
    dev.flush().unwrap();
    assert_eq!(&buf[..1024], &[7; 1024]);
    assert_eq!(&buf[1024..], &[9; 512]);
    assert_eq!(
        counts.get(),
        IoCounts {
            read_calls: 1,
            write_calls: 2,
            read_bytes: 1536,
            write_bytes: 1536,
            flush_calls: 1,
            max_read_bytes: 1536,
            max_write_bytes: 1024,
        }
    );
    counts.set(IoCounts::default());
    dev.read_blocks(BlockIndex::new(1), &mut [0; 512]).unwrap();
    assert_eq!(counts.get().read_bytes, 512);
    assert_eq!(counts.get().write_calls, 0);
}

#[derive(Debug)]
struct Refusing;

impl ErrorType for Refusing {
    type Error = Infallible;
}

impl BlockDevice for Refusing {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        4
    }
    fn max_block_count(&self) -> u64 {
        8
    }
    fn disk_offset(&self) -> u64 {
        4096
    }
    fn read_blocks(&mut self, _: BlockIndex, _: &mut [u8]) -> Result<(), Error<Infallible>> {
        Err(Error::new(ErrorKind::Io, "read refused"))
    }
    fn flush(&mut self) -> Result<(), Error<Infallible>> {
        Err(Error::new(ErrorKind::Io, "flush refused"))
    }
}

#[test]
fn preserves_device_properties_and_errors_without_counting_failed_io() {
    let counts = Cell::new(IoCounts::default());
    let mut dev = Counted {
        inner: Refusing,
        counts: &counts,
        written_blocks: None,
    };
    assert_eq!(dev.block_size().get(), 512);
    assert_eq!(dev.block_count(), 4);
    assert_eq!(dev.max_block_count(), 8);
    assert_eq!(dev.disk_offset(), 4096);
    assert!(!dev.writable());
    assert_eq!(
        dev.read_blocks(BlockIndex::new(0), &mut [0; 512])
            .unwrap_err()
            .kind(),
        ErrorKind::Io
    );
    assert_eq!(
        dev.write_blocks(BlockIndex::new(0), &[0; 512])
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(dev.flush().unwrap_err().kind(), ErrorKind::Io);
    assert_eq!(counts.get(), IoCounts::default());
}

#[test]
fn classifies_repeated_and_multiblock_writes_using_the_final_root_chain() {
    use hadris_fat::sync::format;
    use hadris_fat::{FatKind, FatOptions};

    let mut inner = MemDevice::new(vec![0; 64 << 20], BlockSize::new(512).unwrap());
    let geo = format(&mut inner, &FatOptions::new().with_kind(FatKind::Fat32)).unwrap();
    assert_eq!(geo.cluster_size(), 512);
    let counts = Cell::new(IoCounts::default());
    let written_blocks = vec![Cell::new(0); inner.block_count() as usize];
    let mut dev = Counted {
        inner,
        counts: &counts,
        written_blocks: Some(&written_blocks),
    };
    let mut boot = [0; 512];
    dev.inner
        .read_blocks(BlockIndex::new(0), &mut boot)
        .unwrap();
    let mut write = |at, data: &[u8]| dev.write_blocks(BlockIndex::new(at / 512), data).unwrap();
    write(0, &[0; 512]);
    write(geo.fat_copy(1) - 512, &[0; 1024]);
    write(geo.data_start(), &[0; 1024]);
    write(geo.cluster_offset(4).unwrap(), &[0; 1024]);
    write(geo.cluster_offset(4).unwrap(), &[0; 512]);
    write(geo.fs_info_sector().unwrap() as u64 * 512, &[0; 512]);
    let mut image = dev.inner.into_inner();
    image[..512].copy_from_slice(&boot);
    for copy in 0..geo.fat_count() {
        for (cluster, value) in [(2, 3), (3, FatKind::Fat32.end_of_chain())] {
            let at = (geo.fat_copy(copy) + FatKind::Fat32.entry_offset(cluster)) as usize;
            FatKind::Fat32.encode(cluster, value, &mut image[at..at + 4]);
        }
    }
    let writes = WriteBreakdown::classify(&image, &written_blocks);
    assert_eq!(
        writes,
        WriteBreakdown {
            fat_bytes: [512, 512],
            directory_bytes: 1024,
            data_bytes: 1536,
            fs_info_bytes: 512,
            other_bytes: 512,
        }
    );
    assert_eq!(writes.total(), counts.get().write_bytes);
}

#[test]
fn embedded_single_cluster_append_combines_fat_updates() {
    use hadris_fat::embedded::MountToken;
    use hadris_fat::embedded::sync::Fat;
    use hadris_fat::sync::format;
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::OpenOptions;

    for (kind, size, expected) in [
        (FatKind::Fat12, 2 << 20, 3),
        (FatKind::Fat16, 16 << 20, 3),
        (FatKind::Fat32, 64 << 20, 3),
    ] {
        let mut inner = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        let geo = format(&mut inner, &FatOptions::new().with_kind(kind)).unwrap();
        let counts = Cell::new(IoCounts::default());
        let dev = Counted {
            inner,
            counts: &counts,
            written_blocks: None,
        };
        let mut token = MountToken::new();
        let mut fs: Fat<_> = Fat::mount(dev, &mut token).unwrap();
        let file = fs
            .open(fs.root(), "LOG.BIN", OpenOptions::new().write().create())
            .unwrap();
        fs.write(&file, &vec![7; geo.cluster_size() as usize])
            .unwrap();
        counts.set(IoCounts::default());
        fs.write(&file, &[9]).unwrap();
        assert_eq!(counts.get().write_calls, expected, "{kind:?}");
        assert_eq!(counts.get().write_bytes, expected * 512, "{kind:?}");
        fs.close(file).unwrap();
        fs.unmount().unwrap();
    }
}

#[test]
fn hosted_single_cluster_append_combines_fat_updates() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};

    for (kind, size, expected) in [
        (FatKind::Fat12, 2 << 20, 3),
        (FatKind::Fat16, 16 << 20, 3),
        (FatKind::Fat32, 64 << 20, 3),
    ] {
        let mut inner = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        let geo = format(&mut inner, &FatOptions::new().with_kind(kind)).unwrap();
        let counts = Cell::new(IoCounts::default());
        let dev = Counted {
            inner,
            counts: &counts,
            written_blocks: None,
        };
        let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
        let file = fs
            .create(fs.root(), Name::new("LOG.BIN"), &SetAttr::new())
            .unwrap();
        let cluster = geo.cluster_size() as usize;
        fs.write(file, 0, &vec![7; cluster]).unwrap();
        fs.close(file).unwrap();
        counts.set(IoCounts::default());
        fs.write(file, cluster as u64, &[9]).unwrap();
        assert_eq!(counts.get().write_calls, expected, "{kind:?}");
        assert_eq!(counts.get().write_bytes, expected * 512, "{kind:?}");
        fs.close(file).unwrap();
        fs.unmount().unwrap();
    }
}

#[test]
fn hosted_multi_cluster_append_combines_tail_and_allocation_group() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};
    for (kind, size) in [
        (FatKind::Fat12, 2 << 20),
        (FatKind::Fat16, 16 << 20),
        (FatKind::Fat32, 64 << 20),
    ] {
        let mut inner = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        let geo = format(&mut inner, &FatOptions::new().with_kind(kind)).unwrap();
        let counts = Cell::new(IoCounts::default());
        let dev = Counted {
            inner,
            counts: &counts,
            written_blocks: None,
        };
        let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
        let file = fs
            .create(fs.root(), Name::new("LOG.BIN"), &SetAttr::new())
            .unwrap();
        let cluster = geo.cluster_size() as usize;
        fs.write(file, 0, &vec![7; cluster]).unwrap();
        fs.close(file).unwrap();
        counts.set(IoCounts::default());
        fs.write(file, cluster as u64, &vec![9; 4 * cluster])
            .unwrap();
        assert_eq!(counts.get().write_calls, 3, "{kind:?}");
        assert_eq!(
            counts.get().write_bytes,
            1024 + 4 * cluster as u64,
            "{kind:?}"
        );
        fs.unmount().unwrap();
    }
}

#[test]
fn hosted_insertion_reuses_the_planners_directory_position() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};

    let mut inner = MemDevice::new(vec![0; 64 << 20], BlockSize::new(512).unwrap());
    format(
        &mut inner,
        &FatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_cluster_size(512),
    )
    .unwrap();
    let counts = Cell::new(IoCounts::default());
    let dev = Counted {
        inner,
        counts: &counts,
        written_blocks: None,
    };
    let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
    let root = fs.root();
    for i in 0..1000 {
        let node = fs
            .create(root, Name::new(&format!("F{i:07}.TXT")), &SetAttr::new())
            .unwrap();
        fs.forget(node, 1);
    }
    counts.set(IoCounts::default());
    let node = fs
        .create(root, Name::new("LAST.TXT"), &SetAttr::new())
        .unwrap();
    assert!(counts.get().read_calls <= 125, "{:?}", counts.get());
    fs.forget(node, 1);
    fs.unmount().unwrap();
}

#[test]
fn bulk_short_name_creation_avoids_quadratic_directory_reads() {
    use hadris_fat::sync::write;
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{Content, Node, Tree};

    let mut tree = Tree::new();
    for i in 0..1000 {
        tree.insert(
            format!("F{i:07}.TXT"),
            Node::file(Content::bytes(b"fixture")),
        )
        .unwrap();
    }
    let counts = Cell::new(IoCounts::default());
    let mut dev = Counted {
        inner: MemDevice::new(vec![0; 64 << 20], BlockSize::new(512).unwrap()),
        counts: &counts,
        written_blocks: None,
    };
    write(
        &mut dev,
        &tree,
        &FatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_cluster_size(512),
    )
    .unwrap();
    assert!(counts.get().read_calls < 6000, "{:?}", counts.get());
}

#[test]
fn first_small_writes_skip_reads_only_for_exclusive_device_blocks() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};

    for (block, cluster, max_reads) in [(512, 512, 3), (4096, 512, 4), (4096, 4096, 2)] {
        let mut inner = MemDevice::new(vec![0xa5; 2 << 20], BlockSize::new(block).unwrap());
        format(
            &mut inner,
            &FatOptions::new()
                .with_kind(FatKind::Fat12)
                .with_sector_size(512)
                .with_cluster_size(cluster),
        )
        .unwrap();
        let counts = Cell::new(IoCounts::default());
        let dev = Counted {
            inner,
            counts: &counts,
            written_blocks: None,
        };
        let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
        let root = fs.root();
        let neighbour = fs
            .create(root, Name::new("OTHER.BIN"), &SetAttr::new())
            .unwrap();
        fs.write(neighbour, 0, &vec![0x7c; cluster as usize])
            .unwrap();
        let file = fs
            .create(root, Name::new("SMALL.BIN"), &SetAttr::new())
            .unwrap();
        counts.set(IoCounts::default());
        fs.write(file, 0, b"original").unwrap();
        assert!(counts.get().read_calls <= max_reads, "{:?}", counts.get());
        fs.write(file, 1, b"XX").unwrap();
        let mut data = [0; 8];
        fs.read(file, 0, &mut data).unwrap();
        assert_eq!(&data, b"oXXginal");
        let mut other = vec![0; cluster as usize];
        fs.read(neighbour, 0, &mut other).unwrap();
        assert_eq!(other, vec![0x7c; cluster as usize]);
        let sparse = fs
            .create(root, Name::new("SPARSE.BIN"), &SetAttr::new())
            .unwrap();
        fs.write(sparse, 7, b"end").unwrap();
        let mut sparse_data = [0; 10];
        fs.read(sparse, 0, &mut sparse_data).unwrap();
        assert_eq!(&sparse_data[..7], &[0; 7]);
        assert_eq!(&sparse_data[7..], b"end");
        fs.unmount().unwrap();
    }
}

#[test]
fn allocation_fails_without_a_scan_once_no_cluster_is_free() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};

    for (kind, size) in [(FatKind::Fat16, 16 << 20), (FatKind::Fat32, 64 << 20)] {
        let mut inner = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        format(
            &mut inner,
            &FatOptions::new().with_kind(kind).with_cluster_size(512),
        )
        .unwrap();
        let counts = Cell::new(IoCounts::default());
        let dev = Counted {
            inner,
            counts: &counts,
            written_blocks: None,
        };
        let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
        let file = fs
            .create(fs.root(), Name::new("FILL.BIN"), &SetAttr::new())
            .unwrap();
        let mut end = 0u64;
        for chunk in [1 << 20, 512] {
            let data = vec![1u8; chunk];
            loop {
                match fs.write(file, end, &data) {
                    Ok(n) if n > 0 => end += n as u64,
                    Ok(_) => break,
                    Err(err) => {
                        assert_eq!(err.kind(), ErrorKind::NoSpace, "{kind:?}");
                        break;
                    }
                }
            }
        }
        counts.set(IoCounts::default());
        let err = fs.write(file, end + 512, &[1]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NoSpace, "{kind:?}");
        assert!(counts.get().read_calls < 16, "{kind:?} {:?}", counts.get());
        fs.close(file).unwrap();
        fs.unmount().unwrap();
    }
}

#[test]
fn gap_fills_write_many_blocks_per_call() {
    use hadris_fat::sync::{FatFs, format};
    use hadris_fat::{FatKind, FatOptions};
    use hadris_fs::{MountOptions, Name, SetAttr};

    let mut inner = MemDevice::new(vec![0xa5; 64 << 20], BlockSize::new(512).unwrap());
    format(
        &mut inner,
        &FatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_cluster_size(512),
    )
    .unwrap();
    let counts = Cell::new(IoCounts::default());
    let dev = Counted {
        inner,
        counts: &counts,
        written_blocks: None,
    };
    let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
    let file = fs
        .create(fs.root(), Name::new("SPARSE.BIN"), &SetAttr::new())
        .unwrap();
    counts.set(IoCounts::default());
    fs.write(file, 1 << 20, b"end").unwrap();
    assert!(counts.get().write_calls < 100, "{:?}", counts.get());
    assert!(
        counts.get().max_write_bytes >= 32 << 10,
        "{:?}",
        counts.get()
    );
    let mut data = vec![1; (1 << 20) + 3];
    let mut done = 0;
    while done < data.len() {
        done += fs.read(file, done as u64, &mut data[done..]).unwrap();
    }
    assert!(data[..1 << 20].iter().all(|&b| b == 0));
    assert_eq!(&data[1 << 20..], b"end");
    fs.close(file).unwrap();
    fs.unmount().unwrap();
}

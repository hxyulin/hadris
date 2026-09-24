use hadris_fat_raw::io::{BlockBuf, ChainPos, DirStart, DirWalk, Held, sync as io};
use hadris_fat_raw::layout::{self, BootFields, Request};
use hadris_fat_raw::{ENTRY_FREE, FatKind, ShortEntry, Slot};
use hadris_fs::{DateTime, ErrorKind};
use hadris_storage::{BlockSize, MemDevice};

const MIB: usize = 1 << 20;

fn format(image: &mut [u8], kind: FatKind) -> (MemDevice<&mut [u8]>, BlockBuf<[u8; 512]>) {
    let mut dev = MemDevice::new(image, BlockSize::new(512).unwrap());
    let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
    let sectors = dev_len(&dev) / 512;
    let plan = layout::plan(&Request::new(512, sectors).with_kind(kind)).unwrap();
    let fields = BootFields::new().with_label(*b"RAW IO     ");
    let now = DateTime::from_unix_seconds(1_700_000_000).unwrap();
    let geo = io::mkfs(&mut dev, &mut block, &plan, &fields, now).unwrap();
    assert_eq!(io::read_geometry(&mut dev, &mut block).unwrap(), geo);
    (dev, block)
}

fn dev_len(dev: &MemDevice<&mut [u8]>) -> u64 {
    use hadris_storage::sync::BlockDevice;
    dev.block_count() * 512
}

#[test]
fn block_buf_sizes() {
    assert!(BlockBuf::<[u8; 512]>::new(0).is_none());
    assert!(BlockBuf::<[u8; 512]>::new(1024).is_none());
    let block = BlockBuf::<[u8; 4096]>::new(512).unwrap();
    assert_eq!(block.block_size(), 512);
    assert_eq!(block.contents().len(), 512);
    assert_eq!(block.cached(), None);
}

#[test]
fn allocate_and_free_keep_every_copy_equal() {
    for (kind, size) in [
        (FatKind::Fat12, 2 * MIB),
        (FatKind::Fat16, 32 * MIB),
        (FatKind::Fat32, 64 * MIB),
    ] {
        let mut image = vec![0u8; size];
        let (mut dev, mut block) = format(&mut image, kind);
        let geo = io::read_geometry(&mut dev, &mut block).unwrap();
        let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
        let total = io::count_free(&mut dev, &mut block, &mut fat).unwrap();

        let mut held = Held::NONE;
        let first = io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), 40).unwrap();
        assert_eq!(held.head(), first, "{kind:?}");
        assert_eq!(held.extra(), 0);
        assert_eq!(fat.free_clusters(), Some(total - 40));
        assert_eq!(fat.unmirrored(), None);

        let mut chain = vec![first];
        while let Some(next) = io::next(&mut dev, &mut block, &fat, *chain.last().unwrap()).unwrap()
        {
            chain.push(next);
        }
        assert_eq!(chain.len(), 40);
        for &cluster in &chain {
            let a = io::get_copy(&mut dev, &mut block, &fat, 0, cluster).unwrap();
            let b = io::get_copy(&mut dev, &mut block, &fat, 1, cluster).unwrap();
            assert_eq!(a, b);
        }
        let end = io::walk(&mut dev, &mut block, &fat, first, ChainPos::NONE, 100).unwrap();
        assert_eq!((end.index(), end.cluster()), (39, chain[39]));

        io::free_chain(&mut dev, &mut block, &mut fat, &mut held, first).unwrap();
        assert_eq!(held, Held::NONE);
        assert_eq!(fat.free_clusters(), Some(total));
        assert_eq!(
            io::count_free(&mut dev, &mut block, &mut fat).unwrap(),
            total
        );
        assert_eq!(
            io::get(&mut dev, &mut block, &fat, 1).unwrap_err().kind(),
            ErrorKind::Corrupt
        );

        assert!(fat.fs_info_dirty() || kind != FatKind::Fat32);
        io::write_fs_info(&mut dev, &mut block, &mut fat).unwrap();
        let again = io::read_fat(&mut dev, &mut block, geo).unwrap();
        if kind == FatKind::Fat32 {
            assert_eq!(again.free_clusters(), Some(total));
        }
    }
}

#[test]
fn slots_round_trip() {
    let mut image = vec![0u8; 32 * MIB];
    let (mut dev, mut block) = format(&mut image, FatKind::Fat16);
    let geo = io::read_geometry(&mut dev, &mut block).unwrap();
    let fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
    let root = fat.root();
    assert!(matches!(root, DirStart::Fixed { slots: 512, .. }));

    let mut walk = DirWalk::new(root);
    let at = io::slot_offset(&mut dev, &mut block, &fat, &mut walk, 0)
        .unwrap()
        .unwrap();
    let Slot::Short(label) = io::read_slot(&mut dev, &mut block, at).unwrap() else {
        panic!("no label entry");
    };
    assert_eq!(label.name(), *b"RAW IO     ");

    let entries = [
        ShortEntry::new(*b"A       TXT", 0),
        ShortEntry::new(*b"B       TXT", 0),
    ];
    let mut written = 0;
    io::write_slots(
        &mut dev,
        &mut block,
        &fat,
        &mut walk,
        1,
        2,
        |i| entries[i as usize].encode(),
        &mut written,
    )
    .unwrap();
    assert_eq!(written, 2);
    let at = io::slot_offset(&mut dev, &mut block, &fat, &mut walk, 2)
        .unwrap()
        .unwrap();
    assert!(
        matches!(io::read_slot(&mut dev, &mut block, at).unwrap(), Slot::Short(e) if e.name() == *b"B       TXT")
    );

    io::clear_slots(&mut dev, &mut block, &fat, root, 1, 3).unwrap();
    let mut raw = [0u8; 1];
    io::read_bytes(&mut dev, &mut block, at, &mut raw).unwrap();
    assert_eq!(raw[0], ENTRY_FREE);
    assert_eq!(
        io::slot_offset(&mut dev, &mut block, &fat, &mut walk, 512).unwrap(),
        None
    );
}

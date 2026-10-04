use hadris_fat_raw::io::{BlockBuf, ChainPos, DirStart, DirWalk, Held, sync as io};
use hadris_fat_raw::layout::{self, BootFields, Request};
use hadris_fat_raw::{
    Detail, ENTRY_FREE, FatKind, LongEntry, NT_LOWER_BASE, ShortEntry, Slot, lfn_checksum,
};
use hadris_fs::{DateTime, ErrorKind};
use hadris_storage::{BlockSize, MemDevice};

const MIB: usize = 1 << 20;

fn format(image: &mut [u8], kind: FatKind) -> (MemDevice<&mut [u8]>, BlockBuf<[u8; 512]>) {
    format_copies(image, kind, 2)
}

fn format_copies(
    image: &mut [u8],
    kind: FatKind,
    copies: u8,
) -> (MemDevice<&mut [u8]>, BlockBuf<[u8; 512]>) {
    let mut dev = MemDevice::new(image, BlockSize::new(512).unwrap());
    let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
    let sectors = dev_len(&dev) / 512;
    let plan = layout::plan(
        &Request::new(512, sectors)
            .with_kind(kind)
            .with_fat_count(copies),
    )
    .unwrap();
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
fn allocate_after_links_forward_backward_and_across_blocks() {
    for (kind, size) in [
        (FatKind::Fat12, 2 * MIB),
        (FatKind::Fat16, 32 * MIB),
        (FatKind::Fat32, 64 * MIB),
    ] {
        let mut image = vec![0; size];
        let (mut dev, mut block) = format(&mut image, kind);
        let geo = io::read_geometry(&mut dev, &mut block).unwrap();
        let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
        let free = io::count_free(&mut dev, &mut block, &mut fat).unwrap();
        let first = io::allocate(&mut dev, &mut block, &mut fat, None).unwrap();
        let mut tail = first;
        for _ in 0..260 {
            let mut held = Held::NONE;
            let added =
                io::allocate_after(&mut dev, &mut block, &mut fat, &mut held, tail).unwrap();
            assert_eq!(held, Held::new(added, 0));
            for copy in 0..geo.fat_count() {
                assert_eq!(
                    io::get_copy(&mut dev, &mut block, &fat, copy, tail).unwrap() & kind.mask(),
                    added
                );
                assert!(
                    kind.next(
                        io::get_copy(&mut dev, &mut block, &fat, copy, added).unwrap(),
                        geo.max_cluster()
                    )
                    .unwrap()
                    .is_none()
                );
            }
            tail = added;
        }
        assert_eq!(fat.free_clusters(), Some(free - 261));
        let mut empty = Held::NONE;
        assert_eq!(
            io::allocate_after(&mut dev, &mut block, &mut fat, &mut empty, first)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        let mut held = Held::new(first, 0);
        assert_eq!(
            io::allocate_after(&mut dev, &mut block, &mut fat, &mut held, tail)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        io::set(&mut dev, &mut block, &mut fat, first, 0).unwrap();
        let mut again = hadris_fat_raw::io::Fat::new(geo);
        let mut held = Held::NONE;
        let added = io::allocate_after(&mut dev, &mut block, &mut again, &mut held, tail).unwrap();
        assert_eq!(added, first);
        assert_eq!(
            io::next(&mut dev, &mut block, &again, tail).unwrap(),
            Some(first)
        );
        io::set(&mut dev, &mut block, &mut again, first, 0).unwrap();
        let tail = first + 1;
        io::set(&mut dev, &mut block, &mut again, tail, kind.end_of_chain()).unwrap();
        let mut again = hadris_fat_raw::io::Fat::new(geo);
        let mut held = Held::NONE;
        assert_eq!(
            io::allocate_after(&mut dev, &mut block, &mut again, &mut held, tail).unwrap(),
            first
        );
        for copy in 0..geo.fat_count() {
            assert_eq!(
                io::get_copy(&mut dev, &mut block, &again, copy, tail).unwrap() & kind.mask(),
                first
            );
            assert!(kind.is_end_of_chain(
                io::get_copy(&mut dev, &mut block, &again, copy, first).unwrap() & kind.mask()
            ));
        }
    }
}

#[test]
fn allocate_after_preserves_reserved_bits_and_writes_only_the_active_fat() {
    let mut image = vec![0; 64 * MIB];
    let (mut dev, mut block) = format(&mut image, FatKind::Fat32);
    io::write_bytes(&mut dev, &mut block, 40, &[0x81, 0]).unwrap();
    let geo = io::read_geometry(&mut dev, &mut block).unwrap();
    assert_eq!(geo.active_fat(), 1);
    assert!(!geo.mirrored());
    let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
    let tail = io::allocate(&mut dev, &mut block, &mut fat, None).unwrap();
    let candidate = fat.next_free();
    for (cluster, stored) in [(tail, 0xAFFF_FFF8u32), (candidate, 0xB000_0000)] {
        let at = geo.fat_copy(1) + FatKind::Fat32.entry_offset(cluster as u64);
        io::write_bytes(&mut dev, &mut block, at, &stored.to_le_bytes()).unwrap();
    }
    let mut held = Held::NONE;
    let added = io::allocate_after(&mut dev, &mut block, &mut fat, &mut held, tail).unwrap();
    assert_eq!(added, candidate);
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 1, tail).unwrap(),
        0xA000_0000 | added
    );
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 1, added).unwrap(),
        0xBFFF_FFF8
    );
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 0, tail).unwrap(),
        0
    );
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 0, added).unwrap(),
        0
    );
    assert_eq!(fat.unmirrored(), None);
}

#[test]
fn allocate_run_after_preserves_reserved_bits_on_the_active_fat() {
    let mut image = vec![0; 64 * MIB];
    let (mut dev, mut block) = format(&mut image, FatKind::Fat32);
    io::write_bytes(&mut dev, &mut block, 40, &[0x81, 0]).unwrap();
    let geo = io::read_geometry(&mut dev, &mut block).unwrap();
    let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
    let tail = io::allocate(&mut dev, &mut block, &mut fat, None).unwrap();
    let first = fat.next_free();
    for (cluster, stored) in [(tail, 0xAFFF_FFF8u32), (first, 0xB000_0000)] {
        let at = geo.fat_copy(1) + FatKind::Fat32.entry_offset(cluster as u64);
        io::write_bytes(&mut dev, &mut block, at, &stored.to_le_bytes()).unwrap();
    }
    let mut held = Held::NONE;
    assert_eq!(
        io::allocate_run_after(&mut dev, &mut block, &mut fat, &mut held, tail, 4).unwrap(),
        first
    );
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 1, tail).unwrap(),
        0xA000_0000 | first
    );
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 1, first).unwrap(),
        0xB000_0000 | (first + 1)
    );
    for cluster in tail..first + 4 {
        assert_eq!(
            io::get_copy(&mut dev, &mut block, &fat, 0, cluster).unwrap(),
            0
        );
    }
    assert_eq!(
        io::get_copy(&mut dev, &mut block, &fat, 1, first + 3).unwrap(),
        FatKind::Fat32.end_of_chain()
    );
    assert_eq!(held.head(), first);
    assert_eq!(held.extra(), 0);
    assert_eq!(fat.unmirrored(), None);
}

#[test]
fn allocation_wraps_past_occupied_clusters_and_stops_on_full_volumes() {
    for copies in [1, 2] {
        let mut image = vec![0; 64 * MIB];
        let (mut dev, mut block) = format_copies(&mut image, FatKind::Fat32, copies);
        let geo = io::read_geometry(&mut dev, &mut block).unwrap();
        let max = geo.max_cluster();
        let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
        io::set(
            &mut dev,
            &mut block,
            &mut fat,
            max - 1,
            FatKind::Fat32.end_of_chain(),
        )
        .unwrap();
        io::write_fs_info(&mut dev, &mut block, &mut fat).unwrap();
        let info = fat.fs_info().unwrap();
        io::write_bytes(&mut dev, &mut block, info + 492, &(max - 2).to_le_bytes()).unwrap();
        let before = dev.into_inner().to_vec();
        for grouped in [false, true] {
            let mut working = before.clone();
            let mut dev = MemDevice::new(working.as_mut_slice(), BlockSize::new(512).unwrap());
            let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
            let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
            let free = fat.free_clusters().unwrap();
            let mut held = Held::NONE;
            let expected = [max - 2, max, 3, 4, 5, 6, 7, 8];
            if grouped {
                let head =
                    io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), 8).unwrap();
                assert_eq!(head, expected[0]);
                let mut chain = vec![head];
                while let Some(next) =
                    io::next(&mut dev, &mut block, &fat, *chain.last().unwrap()).unwrap()
                {
                    chain.push(next);
                    assert!(chain.len() <= expected.len());
                }
                assert_eq!(chain, expected);
            } else {
                for cluster in expected {
                    assert_eq!(
                        io::allocate(&mut dev, &mut block, &mut fat, None).unwrap(),
                        cluster
                    );
                }
            }
            assert_eq!(fat.next_free(), 9);
            assert_eq!(
                io::count_free(&mut dev, &mut block, &mut fat).unwrap(),
                free - 8
            );
            let mut held = Held::NONE;
            assert_eq!(
                io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), free - 7)
                    .unwrap_err()
                    .kind(),
                ErrorKind::NoSpace
            );
            assert_eq!(held, Held::NONE);
            assert_eq!(fat.next_free(), 9);
            let head = io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), free - 8)
                .unwrap();
            let mut empty = Held::NONE;
            assert_eq!(
                io::allocate(&mut dev, &mut block, &mut fat, Some(&mut empty))
                    .unwrap_err()
                    .kind(),
                ErrorKind::NoSpace
            );
            assert_eq!(empty, Held::NONE);
            assert_eq!(fat.free_clusters(), Some(0));
            for cluster in expected {
                for copy in 0..copies {
                    assert_eq!(
                        io::get_copy(&mut dev, &mut block, &fat, copy, cluster).unwrap(),
                        io::get(&mut dev, &mut block, &fat, cluster).unwrap()
                    );
                }
            }
            io::free_chain(&mut dev, &mut block, &mut fat, &mut held, head).unwrap();
            assert_eq!(
                io::count_free(&mut dev, &mut block, &mut fat).unwrap(),
                free - 8
            );
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

#[test]
fn check_paths_decode_stored_names() {
    let mut image = vec![0u8; 2 * MIB];
    let (mut dev, mut block) = format(&mut image, FatKind::Fat12);
    let geo = io::read_geometry(&mut dev, &mut block).unwrap();
    let fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
    let short = *b"A~1     TXT";
    let mut units = [0xFFFFu16; 13];
    units[..4].copy_from_slice(&[u16::from(b'a'), 0xD800, u16::from(b'b'), 0]);
    let long = LongEntry::new(0x41, lfn_checksum(&short), &units);
    let sized = |name: [u8; 11], case: u8| {
        let mut entry = ShortEntry::new(name, 0);
        entry.set_size(1);
        entry.set_nt_case(case);
        entry.encode()
    };
    let slots = [
        long.encode(),
        sized(short, 0),
        sized(*b"\xE9T      TXT", 0),
        sized(*b"LOW     TXT", NT_LOWER_BASE),
    ];
    let mut walk = DirWalk::new(fat.root());
    let mut written = 0;
    io::write_slots(
        &mut dev,
        &mut block,
        &fat,
        &mut walk,
        1,
        4,
        |i| slots[i as usize],
        &mut written,
    )
    .unwrap();

    let mut found = Vec::new();
    let mut scratch = [0u8; 1536];
    let report = io::check(&mut dev, &mut scratch, |finding| {
        let detail = Detail::from_code(finding.detail()).unwrap();
        found.push((
            detail,
            finding.path().unwrap().to_vec(),
            finding.to_string(),
        ));
    })
    .unwrap();
    assert_eq!(report.findings(), 3);
    let paths: Vec<&[u8]> = found.iter().map(|(_, path, _)| path.as_slice()).collect();
    assert_eq!(paths, [&b"/a\xEF\xBF\xBDb"[..], b"/\xE9T.TXT", b"/low.TXT"]);
    assert!(
        found
            .iter()
            .all(|(detail, ..)| *detail == Detail::SizeMismatch)
    );
    assert!(found[1].2.contains("/\\xe9T.TXT"), "{}", found[1].2);
}

#[test]
fn allocate_run_after_links_batches_across_fat_blocks_and_checks_inputs() {
    for (kind, size) in [
        (FatKind::Fat12, 2 * MIB),
        (FatKind::Fat16, 32 * MIB),
        (FatKind::Fat32, 64 * MIB),
    ] {
        for copies in [1, 2] {
            let mut image = vec![0; size];
            let (mut dev, mut block) = format_copies(&mut image, kind, copies);
            let geo = io::read_geometry(&mut dev, &mut block).unwrap();
            let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
            let free = io::count_free(&mut dev, &mut block, &mut fat).unwrap();
            let first = io::allocate_run(&mut dev, &mut block, &mut fat, None, 120).unwrap();
            let mut tail = first;
            while let Some(next) = io::next(&mut dev, &mut block, &fat, tail).unwrap() {
                tail = next;
            }
            let mut held = Held::NONE;
            let head = io::allocate_run_after(&mut dev, &mut block, &mut fat, &mut held, tail, 300)
                .unwrap();
            assert_eq!(held, Held::new(head, 0));
            assert_eq!(
                io::next(&mut dev, &mut block, &fat, tail).unwrap(),
                Some(head)
            );
            let mut cluster = head;
            let mut count = 0;
            loop {
                let stored = io::get(&mut dev, &mut block, &fat, cluster).unwrap();
                for copy in 0..copies {
                    assert_eq!(
                        io::get_copy(&mut dev, &mut block, &fat, copy, cluster).unwrap(),
                        stored
                    );
                }
                count += 1;
                match io::next(&mut dev, &mut block, &fat, cluster).unwrap() {
                    Some(next) => cluster = next,
                    None => break,
                }
            }
            assert_eq!(count, 300);
            assert_eq!(fat.free_clusters(), Some(free - 420));
            let mut empty = Held::NONE;
            assert_eq!(
                io::allocate_run_after(&mut dev, &mut block, &mut fat, &mut empty, cluster, 0)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput
            );
            assert_eq!(empty, Held::NONE);
            assert_eq!(
                io::allocate_run_after(&mut dev, &mut block, &mut fat, &mut empty, tail, 2)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput
            );
            assert_eq!(empty, Held::NONE);
            assert_eq!(
                io::allocate_run_after(&mut dev, &mut block, &mut fat, &mut held, cluster, 2)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput
            );
            assert_eq!(fat.free_clusters(), Some(free - 420));
        }
    }
}

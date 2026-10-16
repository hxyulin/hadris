//! Builds a small APFS container whose filesystem tree has an index root and
//! three leaves, with every child link stored as a virtual object identifier.

use hadris_apfs::types::checksum::fletcher64;

pub const BLOCK: usize = 4096;

/// How the volume's filesystem tree is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Variant {
    /// Ordinary virtual tree with object headers.
    Plain,
    /// Sealed-volume layout: hashed index entries with root-relative child
    /// identifiers and nodes mapped without an object header.
    Sealed,
    /// Nodes are mapped as encrypted.
    Encrypted,
    /// The index root names the same child twice.
    Revisit,
    /// File 0 claims a size of `u64::MAX`.
    HugeFile,
    /// A case- and normalization-sensitive volume with `j_drec_key_t` keys.
    PlainKeys,
    /// A volume flagged sealed whose file extents live in the file extent tree.
    SealedFext,
}

/// The volume incompatible features of [`Variant::Plain`]: normalization
/// insensitivity, as macOS formats volumes, so directory keys are hashed.
pub const DEFAULT_VOLUME_INCOMPAT: u64 = 0x8;
/// Block holding the [`Variant::SealedFext`] file extent tree.
pub const FEXT_TREE_BLOCK: u64 = 19;
pub const VOLUME_OID: u64 = 1026;
pub const FS_ROOT_OID: u64 = 1027;
pub const FS_ROOT_BLOCK: u64 = 6;
pub const IMAGE_BLOCKS: usize = 24;
pub const FILE_COUNT: u64 = 6;

pub fn file_name(index: u64) -> String {
    format!("file{index}.txt")
}

pub fn file_contents(index: u64) -> Vec<u8> {
    format!("contents of file {index}\n").into_bytes()
}

fn put(block: &mut [u8], offset: usize, bytes: &[u8]) {
    block[offset..offset + bytes.len()].copy_from_slice(bytes);
}

fn object(oid: u64, xid: u64, kind: u32) -> Vec<u8> {
    let mut block = vec![0_u8; BLOCK];
    put(&mut block, 8, &oid.to_le_bytes());
    put(&mut block, 16, &xid.to_le_bytes());
    put(&mut block, 24, &kind.to_le_bytes());
    block
}

fn seal(mut block: Vec<u8>) -> Vec<u8> {
    let checksum = fletcher64(&block).unwrap();
    put(&mut block, 0, &checksum.to_le_bytes());
    block
}

type Entry = (Vec<u8>, Vec<u8>);

/// Builds a B-tree node. `tree_flags` and the key/value sizes are only written
/// into the root's `btree_info_t` trailer.
#[allow(clippy::too_many_arguments)]
fn btree_node(
    oid: u64,
    root: bool,
    leaf: bool,
    level: u16,
    fixed: Option<(u32, u32)>,
    tree_flags: u32,
    headerless: bool,
    entries: &[Entry],
) -> Vec<u8> {
    let mut block = if headerless {
        vec![0_u8; BLOCK]
    } else {
        object(oid, 1, if root { 2 } else { 3 })
    };
    let mut flags = 0_u16;
    if root {
        flags |= 1;
    }
    if leaf {
        flags |= 2;
    }
    if fixed.is_some() {
        flags |= 4;
    }
    put(&mut block, 32, &flags.to_le_bytes());
    put(&mut block, 34, &level.to_le_bytes());
    put(&mut block, 36, &(entries.len() as u32).to_le_bytes());
    let toc_entry = if fixed.is_some() { 4 } else { 8 };
    put(&mut block, 40, &0_u16.to_le_bytes());
    put(
        &mut block,
        42,
        &((entries.len() * toc_entry) as u16).to_le_bytes(),
    );

    let data_start = 56;
    let data_len = BLOCK - data_start;
    let value_end = if root { data_len - 40 } else { data_len };
    let key_start = entries.len() * toc_entry;
    let mut key_off = 0;
    let mut value_off = 0;
    for (i, (key, value)) in entries.iter().enumerate() {
        value_off += value.len();
        let toc = data_start + i * toc_entry;
        if fixed.is_some() {
            put(&mut block, toc, &(key_off as u16).to_le_bytes());
            put(&mut block, toc + 2, &(value_off as u16).to_le_bytes());
        } else {
            put(&mut block, toc, &(key_off as u16).to_le_bytes());
            put(&mut block, toc + 2, &(key.len() as u16).to_le_bytes());
            put(&mut block, toc + 4, &(value_off as u16).to_le_bytes());
            put(&mut block, toc + 6, &(value.len() as u16).to_le_bytes());
        }
        put(&mut block, data_start + key_start + key_off, key);
        put(&mut block, data_start + value_end - value_off, value);
        key_off += key.len();
    }
    assert!(
        key_start + key_off <= value_end - value_off,
        "node overflow"
    );

    if root {
        let info = BLOCK - 40;
        let (key_size, value_size) = fixed.unwrap_or((0, 0));
        put(&mut block, info, &tree_flags.to_le_bytes());
        put(&mut block, info + 4, &(BLOCK as u32).to_le_bytes());
        put(&mut block, info + 8, &key_size.to_le_bytes());
        put(&mut block, info + 12, &value_size.to_le_bytes());
    }
    if headerless { block } else { seal(block) }
}

fn omap_entry(oid: u64, xid: u64, address: u64, flags: u32) -> Entry {
    let mut key = oid.to_le_bytes().to_vec();
    key.extend_from_slice(&xid.to_le_bytes());
    let mut value = flags.to_le_bytes().to_vec();
    value.extend_from_slice(&(BLOCK as u32).to_le_bytes());
    value.extend_from_slice(&address.to_le_bytes());
    (key, value)
}

fn fs_key(id: u64, record_type: u64) -> Vec<u8> {
    (id | (record_type << 60)).to_le_bytes().to_vec()
}

fn inode_entry(id: u64, private_id: u64, mode: u16, size: u64) -> Entry {
    let mut value = vec![0_u8; 92];
    put(&mut value, 0, &2_u64.to_le_bytes());
    put(&mut value, 8, &private_id.to_le_bytes());
    put(&mut value, 56, &1_i32.to_le_bytes());
    put(&mut value, 80, &mode.to_le_bytes());
    value.extend_from_slice(&1_u16.to_le_bytes());
    value.extend_from_slice(&0_u16.to_le_bytes());
    value.extend_from_slice(&[8, 0]);
    value.extend_from_slice(&40_u16.to_le_bytes());
    let mut dstream = vec![0_u8; 40];
    put(&mut dstream, 0, &size.to_le_bytes());
    value.extend_from_slice(&dstream);
    (fs_key(id, 3), value)
}

fn drec_entry(parent: u64, name: &str, file_id: u64) -> Entry {
    drec_entry_bytes(parent, name.as_bytes(), file_id, true)
}

/// A directory record for `name`, with a hashed or a plain key.
pub fn drec_entry_bytes(parent: u64, name: &[u8], file_id: u64, hashed: bool) -> Entry {
    let mut key = fs_key(parent, 9);
    if hashed {
        key.extend_from_slice(&((name.len() + 1) as u32).to_le_bytes());
    } else {
        key.extend_from_slice(&((name.len() + 1) as u16).to_le_bytes());
    }
    key.extend_from_slice(name);
    key.push(0);
    let mut value = file_id.to_le_bytes().to_vec();
    value.extend_from_slice(&0_u64.to_le_bytes());
    value.extend_from_slice(&8_u16.to_le_bytes());
    (key, value)
}

fn extent_entry(private_id: u64, length: u64, physical: u64) -> Entry {
    let mut key = fs_key(private_id, 8);
    key.extend_from_slice(&0_u64.to_le_bytes());
    let mut value = length.to_le_bytes().to_vec();
    value.extend_from_slice(&physical.to_le_bytes());
    value.extend_from_slice(&0_u64.to_le_bytes());
    (key, value)
}

fn child_entry(first_key: Vec<u8>, stored_oid: u64, hashed: bool) -> Entry {
    let mut value = stored_oid.to_le_bytes().to_vec();
    if hashed {
        value.extend_from_slice(&[0xab; 32]);
    }
    (first_key, value)
}

/// An index entry of a physical tree: a child's first key and its block.
fn index_entry(first_key: Vec<u8>, child_block: u64) -> Entry {
    (first_key, child_block.to_le_bytes().to_vec())
}

pub fn build_image() -> Vec<u8> {
    build_image_variant(Variant::Plain)
}

/// The filesystem records of files `range`: their root directory entries,
/// inodes and (unless the variant keeps them in the file extent tree) extents.
fn file_records(variant: Variant, range: std::ops::Range<u64>) -> Vec<Entry> {
    let hashed = variant != Variant::PlainKeys;
    let mut records = Vec::new();
    for index in range {
        records.push(drec_entry_bytes(
            2,
            file_name(index).as_bytes(),
            16 + index,
            hashed,
        ));
        let size = if variant == Variant::HugeFile && index == 0 {
            u64::MAX
        } else {
            file_contents(index).len() as u64
        };
        records.push(inode_entry(16 + index, 100 + index, 0o100644, size));
        if variant != Variant::SealedFext {
            records.push(extent_entry(100 + index, BLOCK as u64, 10 + index));
        }
    }
    records
}

pub fn build_image_variant(variant: Variant) -> Vec<u8> {
    let mut records = vec![inode_entry(2, 2, 0o040755, 0)];
    records.extend(file_records(variant, 0..FILE_COUNT));
    build_image_records(variant, records)
}

/// Builds the image with `records` as the filesystem tree's contents.
fn build_image_records(variant: Variant, mut records: Vec<Entry>) -> Vec<u8> {
    let sealed = matches!(variant, Variant::Sealed | Variant::SealedFext);
    let node_flags = match variant {
        Variant::Sealed | Variant::SealedFext => 8,
        Variant::Encrypted => 4,
        _ => 0,
    };
    let volume_incompat = match variant {
        Variant::PlainKeys => 0,
        Variant::SealedFext => DEFAULT_VOLUME_INCOMPAT | 0x20,
        _ => DEFAULT_VOLUME_INCOMPAT,
    };
    let rebase = if sealed { FS_ROOT_OID } else { 0 };
    let tree_flags = if sealed { 0x80 | 0x100 } else { 0 };
    let mut image = vec![0_u8; BLOCK * IMAGE_BLOCKS];
    let mut write = |index: usize, block: Vec<u8>| {
        image[index * BLOCK..(index + 1) * BLOCK].copy_from_slice(&block);
    };

    let mut superblock = object(1, 1, 1);
    put(&mut superblock, 32, b"NXSB");
    put(&mut superblock, 36, &(BLOCK as u32).to_le_bytes());
    put(&mut superblock, 40, &(IMAGE_BLOCKS as u64).to_le_bytes());
    put(&mut superblock, 152, &18_u64.to_le_bytes());
    put(&mut superblock, 160, &1_u64.to_le_bytes());
    put(&mut superblock, 184, &VOLUME_OID.to_le_bytes());
    write(0, seal(superblock));

    let mut omap = object(1, 1, 11);
    put(&mut omap, 48, &2_u64.to_le_bytes());
    write(1, seal(omap));
    write(
        2,
        btree_node(
            2,
            true,
            true,
            0,
            Some((16, 16)),
            0x10,
            false,
            &[omap_entry(VOLUME_OID, 1, 3, 0)],
        ),
    );

    let mut volume = object(VOLUME_OID, 1, 13);
    put(&mut volume, 32, b"APSB");
    put(&mut volume, 56, &volume_incompat.to_le_bytes());
    if variant == Variant::SealedFext {
        put(&mut volume, 1032, &FEXT_TREE_BLOCK.to_le_bytes());
        put(&mut volume, 1040, &0x4000_0002_u32.to_le_bytes());
    }
    put(&mut volume, 128, &4_u64.to_le_bytes());
    put(&mut volume, 136, &FS_ROOT_OID.to_le_bytes());
    put(&mut volume, 704, b"Test");
    write(3, seal(volume));

    let mut volume_omap = object(4, 1, 11);
    put(&mut volume_omap, 48, &5_u64.to_le_bytes());
    write(4, seal(volume_omap));
    // The xid 5 mapping is newer than the volume superblock and must be ignored.
    write(
        5,
        btree_node(
            5,
            true,
            true,
            0,
            Some((16, 16)),
            0x10,
            false,
            &[
                omap_entry(FS_ROOT_OID, 1, FS_ROOT_BLOCK, node_flags),
                omap_entry(1028, 1, 7, node_flags),
                omap_entry(1028, 5, 20, node_flags),
                omap_entry(1029, 1, 8, node_flags),
                omap_entry(1030, 1, 9, node_flags),
            ],
        ),
    );

    let private = |index: u64| 100 + index;
    // The tree holds the records in key order: by object id, then record
    // type (records sharing both keep their order). The leaves split that
    // sequence and each index entry carries its child's first key, as APFS
    // requires for keyed lookups.
    records.sort_by_key(|(key, _)| {
        let raw = u64::from_le_bytes(key[..8].try_into().unwrap());
        (raw & 0x0fff_ffff_ffff_ffff, raw >> 60)
    });
    let leaves: Vec<&[Entry]> = records.chunks(records.len().div_ceil(3)).collect();
    assert_eq!(leaves.len(), 3);
    for (leaf, (block, oid)) in leaves.iter().zip([(7, 1028), (8, 1029), (9, 1030)]) {
        write(
            block,
            btree_node(oid, false, true, 0, None, 0, sealed, leaf),
        );
    }
    write(
        6,
        btree_node(
            FS_ROOT_OID,
            true,
            false,
            1,
            None,
            tree_flags,
            sealed,
            &[
                child_entry(leaves[0][0].0.clone(), 1028 - rebase, sealed),
                child_entry(
                    leaves[1][0].0.clone(),
                    if variant == Variant::Revisit {
                        1028
                    } else {
                        1029
                    } - rebase,
                    sealed,
                ),
                child_entry(leaves[2][0].0.clone(), 1030 - rebase, sealed),
            ],
        ),
    );

    if variant == Variant::SealedFext {
        let fext: Vec<Entry> = (0..FILE_COUNT)
            .map(|index| {
                let mut key = private(index).to_le_bytes().to_vec();
                key.extend_from_slice(&0_u64.to_le_bytes());
                let mut value = (BLOCK as u64).to_le_bytes().to_vec();
                value.extend_from_slice(&(10 + index).to_le_bytes());
                (key, value)
            })
            .collect();
        write(
            FEXT_TREE_BLOCK as usize,
            btree_node(
                FEXT_TREE_BLOCK,
                true,
                true,
                0,
                Some((16, 16)),
                0x10,
                false,
                &fext,
            ),
        );
    }
    for index in 0..FILE_COUNT {
        let mut data = vec![0_u8; BLOCK];
        let contents = file_contents(index);
        data[..contents.len()].copy_from_slice(&contents);
        write(10 + index as usize, data);
    }
    let mut spaceman = object(18, 1, 5);
    put(&mut spaceman, 32, &(BLOCK as u32).to_le_bytes());
    put(&mut spaceman, 48, &(IMAGE_BLOCKS as u64).to_le_bytes());
    put(&mut spaceman, 72, &4_u64.to_le_bytes());
    write(18, seal(spaceman));
    image
}

/// Sets the volume superblock's incompatible feature flags and reseals it.
#[allow(dead_code)]
pub fn set_volume_incompatible_features(image: &mut [u8], flags: u64) {
    let start = 3 * BLOCK;
    let mut volume = image[start..start + BLOCK].to_vec();
    put(&mut volume, 56, &flags.to_le_bytes());
    image[start..start + BLOCK].copy_from_slice(&seal(volume));
}

/// Extents for file `index` with a sparse extent, an uncovered gap and the
/// file's real block at logical offset `2 * BLOCK`.
#[allow(dead_code)]
pub fn holey_extents(index: u64) -> Vec<hadris_apfs::types::FileExtentRecord> {
    let extent = |logical_address: u64, physical_block| {
        let (mut key, value) = extent_entry(100 + index, BLOCK as u64, physical_block);
        put(&mut key, 8, &logical_address.to_le_bytes());
        hadris_apfs::types::FileExtentRecord::parse(&key, &value).unwrap()
    };
    vec![extent(0, 0), extent(2 * BLOCK as u64, 10 + index)]
}

/// What [`holey_extents`] read as, for a file of `2 * BLOCK + len` bytes.
#[allow(dead_code)]
pub fn holey_contents(index: u64) -> Vec<u8> {
    let mut data = vec![0_u8; 2 * BLOCK];
    data.extend(file_contents(index));
    data
}

#[allow(dead_code)]
pub fn use_case_image() -> Vec<u8> {
    let mut linked = inode_entry(16, 100, 0o100644, file_contents(0).len() as u64);
    put(&mut linked.1, 56, &2_i32.to_le_bytes());
    put(
        &mut linked.1,
        16,
        &1_577_836_800_123_456_789_u64.to_le_bytes(),
    );
    put(
        &mut linked.1,
        24,
        &1_577_836_801_000_000_000_u64.to_le_bytes(),
    );
    put(
        &mut linked.1,
        32,
        &1_577_836_802_000_000_000_u64.to_le_bytes(),
    );
    put(
        &mut linked.1,
        40,
        &1_577_836_803_000_000_000_u64.to_le_bytes(),
    );
    put(&mut linked.1, 72, &501_u32.to_le_bytes());
    put(&mut linked.1, 76, &20_u32.to_le_bytes());
    let mut link_entry = drec_entry(2, "link", 22);
    put(&mut link_entry.1, 16, &10_u16.to_le_bytes());
    let target = b"file0.txt\0";
    let xattr_name = "com.apple.fs.symlink";
    let mut key = fs_key(22, 4);
    key.extend_from_slice(&((xattr_name.len() + 1) as u16).to_le_bytes());
    key.extend_from_slice(xattr_name.as_bytes());
    key.push(0);
    let mut value = 2_u16.to_le_bytes().to_vec();
    value.extend_from_slice(&(target.len() as u16).to_le_bytes());
    value.extend_from_slice(target);
    let mut sparse = extent_entry(107, BLOCK as u64, 10);
    put(&mut sparse.0, 8, &(2 * BLOCK as u64).to_le_bytes());
    let mut entries = vec![
        inode_entry(2, 2, 0o040755, 0),
        drec_entry(2, "alias.txt", 16),
        drec_entry(2, "file0.txt", 16),
        drec_entry(2, "file1.txt", 17),
        link_entry,
        drec_entry(2, "sparse.txt", 23),
        linked,
        extent_entry(100, BLOCK as u64, 10),
        inode_entry(17, 101, 0o100644, file_contents(1).len() as u64),
        extent_entry(101, BLOCK as u64, 11),
        inode_entry(22, 22, 0o120777, (target.len() - 1) as u64),
        (key, value),
        inode_entry(
            23,
            107,
            0o100644,
            2 * BLOCK as u64 + file_contents(0).len() as u64,
        ),
        extent_entry(107, BLOCK as u64, 0),
        sparse,
    ];
    entries.extend(file_records(Variant::Plain, 2..FILE_COUNT));
    build_image_records(Variant::Plain, entries)
}

#[allow(dead_code)]
pub fn multi_volume_image() -> Vec<u8> {
    let mut image = build_image();
    let mut container = image[..BLOCK].to_vec();
    put(&mut container, 192, &2048_u64.to_le_bytes());
    image[..BLOCK].copy_from_slice(&seal(container));
    let omap = btree_node(
        2,
        true,
        true,
        0,
        Some((16, 16)),
        0x10,
        false,
        &[omap_entry(VOLUME_OID, 1, 3, 0), omap_entry(2048, 1, 16, 0)],
    );
    image[2 * BLOCK..3 * BLOCK].copy_from_slice(&omap);
    let mut volume = image[3 * BLOCK..4 * BLOCK].to_vec();
    put(&mut volume, 8, &2048_u64.to_le_bytes());
    put(&mut volume, 36, &1_u32.to_le_bytes());
    put(&mut volume, 240, &[1; 16]);
    put(&mut volume, 704, b"Other\0");
    image[16 * BLOCK..17 * BLOCK].copy_from_slice(&seal(volume));
    image
}

#[allow(dead_code)]
pub fn duplicate_volume_names_image() -> Vec<u8> {
    let mut image = multi_volume_image();
    let mut volume = image[16 * BLOCK..17 * BLOCK].to_vec();
    volume[704..710].fill(0);
    put(&mut volume, 704, b"Test");
    image[16 * BLOCK..17 * BLOCK].copy_from_slice(&seal(volume));
    image
}

/// Repeated 300 times, the 600-byte name of `invalid_directory_image("long")`.
#[allow(dead_code)]
pub const LONG_NAME: &str = "\u{e9}";

/// A root directory whose `file0.txt` entry is damaged as `case` says.
/// Files 2 to 5 stay intact.
#[allow(dead_code)]
pub fn invalid_directory_image(case: &str) -> Vec<u8> {
    // Files 2 to 5 are added below; file 0 is damaged, file 1 absent.
    let mut records = vec![
        inode_entry(2, 2, 0o040755, 0),
        drec_entry(2, "file0.txt", if case == "dangling" { 99 } else { 16 }),
        inode_entry(16, 100, 0o100644, file_contents(0).len() as u64),
        extent_entry(
            100,
            BLOCK as u64,
            if case == "extent" {
                IMAGE_BLOCKS as u64
            } else {
                10
            },
        ),
    ];
    match case {
        "dangling" | "extent" => {}
        "utf8" => records[1].0[12] = 0xff,
        "type" => put(&mut records[1].1, 16, &4_u16.to_le_bytes()),
        "duplicate" => records.push(drec_entry(2, "FILE0.TXT", 16)),
        "mode" => put(&mut records[2].1, 80, &0o644_u16.to_le_bytes()),
        "inode" => records[2].1.truncate(40),
        // File 0's inode keeps its field table but loses the data its only
        // extended field declares.
        "dstream" => {
            let value = &mut records[2].1;
            value.truncate(value.len() - 38);
        }
        "long" => records[1] = drec_entry(2, &LONG_NAME.repeat(300), 16),
        "symlink_stream" => {
            let mut link = drec_entry(2, "link", 22);
            put(&mut link.1, 16, &10_u16.to_le_bytes());
            let name = "com.apple.fs.symlink";
            let mut key = fs_key(22, 4);
            key.extend_from_slice(&((name.len() + 1) as u16).to_le_bytes());
            key.extend_from_slice(name.as_bytes());
            key.push(0);
            let mut value = 1_u16.to_le_bytes().to_vec();
            value.extend_from_slice(&0_u16.to_le_bytes());
            value.extend_from_slice(&[0; 16]);
            records.push(link);
            records.push(inode_entry(22, 22, 0o120777, 9));
            records.push((key, value));
        }
        _ => panic!("unknown malformed directory case"),
    }
    records.extend(file_records(Variant::Plain, 2..FILE_COUNT));
    let mut image = build_image_records(Variant::Plain, records);
    if case == "duplicate" {
        set_volume_incompatible_features(&mut image, 1);
    }
    image
}

/// An image whose container object map has an index root in block 2 naming
/// two or three leaves in blocks 20 to 22, so a lookup must descend a level.
#[allow(dead_code)]
pub fn object_map_versions_image(entries: &[(u64, u64, u64, u32)], xid: u64) -> Vec<u8> {
    let mut image = build_image();
    let mut entries: Vec<Entry> = entries
        .iter()
        .map(|&(oid, xid, address, flags)| omap_entry(oid, xid, address, flags))
        .collect();
    entries.sort_by_key(|(key, _)| {
        (
            u64::from_le_bytes(key[..8].try_into().unwrap()),
            u64::from_le_bytes(key[8..16].try_into().unwrap()),
        )
    });
    let leaves: Vec<&[Entry]> = entries.chunks(entries.len().div_ceil(3)).collect();
    assert!(
        (2..=3).contains(&leaves.len()),
        "the object map needs an index level"
    );
    let mut index = Vec::new();
    for (leaf, block) in leaves.iter().zip([20_u64, 21, 22]) {
        let node = btree_node(block, false, true, 0, None, 0, false, leaf);
        image[block as usize * BLOCK..(block as usize + 1) * BLOCK].copy_from_slice(&node);
        index.push(index_entry(leaf[0].0.clone(), block));
    }
    let root = btree_node(2, true, false, 1, Some((16, 16)), 0x10, false, &index);
    image[2 * BLOCK..3 * BLOCK].copy_from_slice(&root);
    let mut superblock = image[..BLOCK].to_vec();
    put(&mut superblock, 16, &xid.to_le_bytes());
    image[..BLOCK].copy_from_slice(&seal(superblock));
    image
}

//! Builds a small APFS container whose filesystem tree has an index root and
//! three leaves, with every child link stored as a virtual object identifier.

use hadris_apfs::types::checksum::fletcher64;

pub const BLOCK: usize = 4096;

/// How the volume's filesystem tree is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
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
}
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
    let mut key = fs_key(parent, 9);
    key.extend_from_slice(&((name.len() + 1) as u32).to_le_bytes());
    key.extend_from_slice(name.as_bytes());
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

pub fn build_image() -> Vec<u8> {
    build_image_variant(Variant::Plain)
}

pub fn build_image_variant(variant: Variant) -> Vec<u8> {
    let sealed = variant == Variant::Sealed;
    let node_flags = match variant {
        Variant::Plain => 0,
        Variant::Sealed => 8,
        Variant::Encrypted => 4,
        Variant::Revisit => 0,
        Variant::HugeFile => 0,
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
    let inode = |index: u64| 16 + index;
    let leaf_records = |range: std::ops::Range<u64>| {
        let mut entries = Vec::new();
        for index in range.clone() {
            entries.push(drec_entry(2, &file_name(index), inode(index)));
        }
        for index in range {
            let size = if variant == Variant::HugeFile && index == 0 {
                u64::MAX
            } else {
                file_contents(index).len() as u64
            };
            entries.push(inode_entry(inode(index), private(index), 0o100644, size));
            entries.push(extent_entry(private(index), BLOCK as u64, 10 + index));
        }
        entries
    };
    let mut first = vec![inode_entry(2, 2, 0o040755, 0)];
    first.extend(leaf_records(0..2));
    write(7, btree_node(1028, false, true, 0, None, 0, sealed, &first));
    write(
        8,
        btree_node(1029, false, true, 0, None, 0, sealed, &leaf_records(2..4)),
    );
    write(
        9,
        btree_node(1030, false, true, 0, None, 0, sealed, &leaf_records(4..6)),
    );
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
                child_entry(fs_key(2, 3), 1028 - rebase, sealed),
                child_entry(
                    drec_entry(2, &file_name(2), 0).0,
                    if variant == Variant::Revisit {
                        1028
                    } else {
                        1029
                    } - rebase,
                    sealed,
                ),
                child_entry(drec_entry(2, &file_name(4), 0).0, 1030 - rebase, sealed),
            ],
        ),
    );

    for index in 0..FILE_COUNT {
        let mut data = vec![0_u8; BLOCK];
        let contents = file_contents(index);
        data[..contents.len()].copy_from_slice(&contents);
        write(10 + index as usize, data);
    }
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
    let extent = |logical_address, physical_block| hadris_apfs::types::FileExtentRecord {
        id: 100 + index,
        logical_address,
        length: BLOCK as u64,
        flags: 0,
        physical_block,
        cryptography_id: 0,
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

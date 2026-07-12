use hadris_cpio::mode::FileType;
use hadris_cpio::read::CpioReader;
use hadris_cpio::write::file_tree::{FileNode, FileTree};
use hadris_cpio::write::{CpioWriteOptions, CpioWriter};

fn write_archive(tree: &FileTree, use_crc: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    let writer = CpioWriter::new(CpioWriteOptions { use_crc });
    writer.write(&mut buf, tree).expect("write failed");
    buf
}

#[test]
fn roundtrip_single_file() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file(
        "hello.txt",
        b"Hello, world!\n".to_vec(),
        0o644,
    ));

    let archive = write_archive(&tree, false);

    let mut reader = CpioReader::new(archive.as_slice());
    let entry = reader
        .next_entry_alloc()
        .unwrap()
        .expect("expected an entry");

    assert_eq!(entry.name_str().unwrap(), "hello.txt");
    assert_eq!(entry.header().permissions(), 0o644);
    assert_eq!(entry.file_type(), FileType::Regular);
    assert_eq!(entry.file_size(), 14);

    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"Hello, world!\n");

    // Next should be None (TRAILER)
    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_directory_with_files() {
    let mut tree = FileTree::new();
    tree.add(FileNode::dir(
        "etc",
        vec![
            FileNode::file("config.cfg", b"key=value\n".to_vec(), 0o644),
            FileNode::file("hosts", b"127.0.0.1 localhost\n".to_vec(), 0o644),
        ],
        0o755,
    ));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    // First: the directory
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "etc");
    assert_eq!(entry.file_type(), FileType::Directory);
    assert_eq!(entry.header().permissions(), 0o755);
    reader.skip_entry_data_owned(&entry).unwrap();

    // Second: etc/config.cfg
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "etc/config.cfg");
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"key=value\n");

    // Third: etc/hosts
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "etc/hosts");
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"127.0.0.1 localhost\n");

    // Trailer
    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_symlink() {
    let mut tree = FileTree::new();
    tree.add(FileNode::symlink("link", "/usr/bin/target"));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "link");
    assert_eq!(entry.file_type(), FileType::Symlink);

    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(core::str::from_utf8(&data).unwrap(), "/usr/bin/target");

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_device_node() {
    let mut tree = FileTree::new();
    tree.add(FileNode::device("null", FileType::CharDevice, 1, 3, 0o666));
    tree.add(FileNode::device("sda", FileType::BlockDevice, 8, 0, 0o660));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "null");
    assert_eq!(entry.file_type(), FileType::CharDevice);
    assert_eq!(entry.header().rdevmajor, 1);
    assert_eq!(entry.header().rdevminor, 3);
    reader.skip_entry_data_owned(&entry).unwrap();

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "sda");
    assert_eq!(entry.file_type(), FileType::BlockDevice);
    assert_eq!(entry.header().rdevmajor, 8);
    assert_eq!(entry.header().rdevminor, 0);
    reader.skip_entry_data_owned(&entry).unwrap();

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_fifo() {
    let mut tree = FileTree::new();
    tree.add(FileNode::fifo("mypipe", 0o644));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "mypipe");
    assert_eq!(entry.file_type(), FileType::Fifo);
    assert_eq!(entry.header().permissions(), 0o644);
    reader.skip_entry_data_owned(&entry).unwrap();

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_hard_link() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file("original", b"data here".to_vec(), 0o644));
    tree.add(FileNode::hard_link("linked", "original"));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    // First entry: original file
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "original");
    assert_eq!(entry.header().nlink, 2); // 1 + 1 hard link
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"data here");

    // Second entry: hard link (same inode, filesize=0)
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "linked");
    assert_eq!(entry.file_size(), 0);
    reader.skip_entry_data_owned(&entry).unwrap();

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_crc_checksum() {
    let mut tree = FileTree::new();
    let contents = b"CRC test data for verification".to_vec();
    tree.add(FileNode::file("crc_test.bin", contents.clone(), 0o644));

    // Write with CRC
    let archive = write_archive(&tree, true);

    let mut reader = CpioReader::new(archive.as_slice());
    let entry = reader.next_entry_alloc().unwrap().unwrap();

    assert_eq!(entry.magic(), hadris_cpio::CpioMagic::NewcCrc);
    assert_eq!(entry.name_str().unwrap(), "crc_test.bin");

    // Verify the CRC value is non-zero
    let expected_crc: u32 = contents.iter().map(|&b| b as u32).sum();
    assert_eq!(entry.header().check, expected_crc);

    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, contents);
}

#[test]
fn roundtrip_offset_based_seek() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file("first.txt", b"first".to_vec(), 0o644));
    tree.add(FileNode::file("second.txt", b"second".to_vec(), 0o644));
    tree.add(FileNode::file("third.txt", b"third".to_vec(), 0o644));

    let archive = write_archive(&tree, false);
    let cursor = hadris_io::Cursor::new(&archive);
    let mut reader = CpioReader::new(cursor);

    // Read first, record second's offset
    let e1 = reader.next_entry_alloc().unwrap().unwrap();
    reader.skip_entry_data_owned(&e1).unwrap();

    let e2 = reader.next_entry_alloc().unwrap().unwrap();
    let second_offset = e2.entry_offset();
    assert_eq!(e2.name_str().unwrap(), "second.txt");
    reader.skip_entry_data_owned(&e2).unwrap();

    // Skip third
    let e3 = reader.next_entry_alloc().unwrap().unwrap();
    reader.skip_entry_data_owned(&e3).unwrap();

    // Seek back to second
    reader.seek_to_entry(second_offset).unwrap();
    let e2_again = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(e2_again.name_str().unwrap(), "second.txt");
    let data = reader.read_entry_data_alloc(&e2_again).unwrap();
    assert_eq!(data, b"second");
}

#[test]
fn no_alloc_reader_with_fixed_buffer() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file(
        "buf_test.txt",
        b"buffer test".to_vec(),
        0o644,
    ));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let mut name_buf = [0u8; 256];
    let entry = reader.next_entry_with_buf(&mut name_buf).unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "buf_test.txt");
    assert_eq!(entry.file_size(), 11);

    let mut data_buf = [0u8; 11];
    reader.read_entry_data(&entry, &mut data_buf).unwrap();
    assert_eq!(&data_buf, b"buffer test");

    assert!(reader.next_entry_with_buf(&mut name_buf).unwrap().is_none());
}

#[test]
fn alignment_padding_edge_cases() {
    // Test names and data at various alignment boundaries
    let mut tree = FileTree::new();

    // Name "a" (1 byte) + NUL = 2 bytes namesize, header+name = 112, pad = 0
    tree.add(FileNode::file("a", b"x".to_vec(), 0o644));

    // Name "ab" (2 bytes) + NUL = 3 bytes namesize, header+name = 113, pad = 3
    tree.add(FileNode::file("ab", b"xy".to_vec(), 0o644));

    // Name "abc" (3 bytes) + NUL = 4 bytes namesize, header+name = 114, pad = 2
    tree.add(FileNode::file("abc", b"xyz".to_vec(), 0o644));

    // Name "abcd" (4 bytes) + NUL = 5 bytes namesize, header+name = 115, pad = 1
    tree.add(FileNode::file("abcd", b"wxyz".to_vec(), 0o644));

    // Data sizes: 1, 2, 3, 4 bytes with corresponding padding
    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    for (expected_name, expected_data) in [
        ("a", b"x" as &[u8]),
        ("ab", b"xy"),
        ("abc", b"xyz"),
        ("abcd", b"wxyz"),
    ] {
        let entry = reader.next_entry_alloc().unwrap().unwrap();
        assert_eq!(entry.name_str().unwrap(), expected_name);
        let data = reader.read_entry_data_alloc(&entry).unwrap();
        assert_eq!(data, expected_data);
    }

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn trailer_detection() {
    // An archive with no entries should still have a TRAILER
    let tree = FileTree::new();
    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());
    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn error_invalid_magic() {
    let mut bad_archive = vec![0u8; 110];
    bad_archive[0..6].copy_from_slice(b"999999");

    let mut reader = CpioReader::new(bad_archive.as_slice());
    let result = reader.next_entry_alloc();
    assert!(result.is_err());

    match result.unwrap_err() {
        hadris_cpio::CpioError::InvalidMagic { found } => {
            assert_eq!(&found, b"999999");
        }
        e => panic!("expected InvalidMagic, got {e}"),
    }
}

#[test]
fn error_truncated_header() {
    let short = b"07070";
    let mut reader = CpioReader::new(short.as_slice());
    let result = reader.next_entry_alloc();
    assert!(result.is_err());
}

#[test]
fn error_buffer_too_small() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file(
        "very_long_filename.txt",
        b"data".to_vec(),
        0o644,
    ));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    // Buffer too small for the filename
    let mut tiny_buf = [0u8; 2];
    let result = reader.next_entry_with_buf(&mut tiny_buf);
    assert!(result.is_err());
}

#[test]
fn roundtrip_all_node_types() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file_with_owner(
        "regular.txt",
        b"content".to_vec(),
        0o755,
        1000,
        1000,
        1234567890,
    ));
    tree.add(FileNode::dir_with_owner(
        "mydir",
        vec![FileNode::file("inner.txt", b"inner".to_vec(), 0o600)],
        0o755,
        0,
        0,
        1234567890,
    ));
    tree.add(FileNode::symlink("mylink", "/target"));
    tree.add(FileNode::device(
        "mynull",
        FileType::CharDevice,
        1,
        3,
        0o666,
    ));
    tree.add(FileNode::fifo("myfifo", 0o644));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    // Regular file
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "regular.txt");
    assert_eq!(entry.file_type(), FileType::Regular);
    assert_eq!(entry.header().uid, 1000);
    assert_eq!(entry.header().gid, 1000);
    assert_eq!(entry.header().mtime, 1234567890);
    assert_eq!(entry.header().permissions(), 0o755);
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"content");

    // Directory
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "mydir");
    assert_eq!(entry.file_type(), FileType::Directory);
    reader.skip_entry_data_owned(&entry).unwrap();

    // Inner file
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "mydir/inner.txt");
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, b"inner");

    // Symlink
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "mylink");
    assert_eq!(entry.file_type(), FileType::Symlink);
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(core::str::from_utf8(&data).unwrap(), "/target");

    // Char device
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "mynull");
    assert_eq!(entry.file_type(), FileType::CharDevice);
    reader.skip_entry_data_owned(&entry).unwrap();

    // FIFO
    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "myfifo");
    assert_eq!(entry.file_type(), FileType::Fifo);
    reader.skip_entry_data_owned(&entry).unwrap();

    // Trailer
    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_empty_file() {
    let mut tree = FileTree::new();
    tree.add(FileNode::file("empty", Vec::new(), 0o644));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.name_str().unwrap(), "empty");
    assert_eq!(entry.file_size(), 0);

    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert!(data.is_empty());

    assert!(reader.next_entry_alloc().unwrap().is_none());
}

#[test]
fn roundtrip_large_data() {
    // Test with data larger than the skip buffer size (256 bytes)
    let large_data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    let mut tree = FileTree::new();
    tree.add(FileNode::file("large.bin", large_data.clone(), 0o644));

    let archive = write_archive(&tree, false);
    let mut reader = CpioReader::new(archive.as_slice());

    let entry = reader.next_entry_alloc().unwrap().unwrap();
    assert_eq!(entry.file_size(), 1024);
    let data = reader.read_entry_data_alloc(&entry).unwrap();
    assert_eq!(data, large_data);
}

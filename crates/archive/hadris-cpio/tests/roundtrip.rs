mod common;

use common::{archive, newc_entry, read_all, read_all_with, trailer};
use hadris_cpio::sync::{CpioReader, CpioWriter};
use hadris_cpio::{CpioOptions, Detail, Format, NewEntry, ReaderOptions};
use hadris_fs::tree::{Content, Tree, WarningKind};
use hadris_fs::{
    DateTime, DeviceKind, DeviceNumber, ErrorKind, FileTimes, FileType, Mode, SetMetadata,
};
use hadris_io::{Cursor, StdIo};

fn sample_tree() -> Tree {
    let mut tree = Tree::new();
    tree.add_file("init", Content::bytes(b"#!/bin/sh\n".to_vec()))
        .unwrap();
    tree.set_metadata(
        "init",
        SetMetadata::new()
            .with_mode(Mode::new(0o755))
            .with_uid(1000)
            .with_gid(100)
            .with_times(
                FileTimes::new().with_modified(DateTime::from_unix_seconds(1_700_000_000).unwrap()),
            ),
    )
    .unwrap();
    tree.add_file("etc/empty", Content::empty()).unwrap();
    tree.add_file("etc/big.bin", Content::bytes(vec![7u8; 70_001]))
        .unwrap();
    tree.add_dir("dev").unwrap();
    tree.add_device("dev/console", DeviceKind::Char, DeviceNumber::new(5, 1))
        .unwrap();
    tree.add_device("dev/sda", DeviceKind::Block, DeviceNumber::new(8, 0))
        .unwrap();
    tree.add_symlink("bin/sh", "busybox").unwrap();
    tree
}

#[test]
fn every_format_reads_back() {
    for format in [Format::Newc, Format::NewcCrc, Format::Odc] {
        let entries = read_all(&archive(&sample_tree(), format)).unwrap();
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "bin",
                "bin/sh",
                "dev",
                "dev/console",
                "dev/sda",
                "etc",
                "etc/big.bin",
                "etc/empty",
                "init"
            ],
            "{format:?}"
        );
        let get = |name: &str| entries.iter().find(|entry| entry.name == name).unwrap();
        let init = get("init");
        assert_eq!(init.mode, 0o100755);
        assert_eq!((init.uid, init.gid, init.mtime), (1000, 100, 1_700_000_000));
        assert_eq!(init.data, b"#!/bin/sh\n");
        assert_eq!(get("bin").file_type, FileType::Dir);
        assert_eq!(get("bin").mode, 0o040755);
        assert_eq!(get("bin").nlink, 2);
        assert_eq!(get("bin/sh").file_type, FileType::Symlink);
        assert_eq!(get("bin/sh").mode, 0o120777);
        assert_eq!(get("bin/sh").data, b"busybox");
        assert_eq!(get("dev/console").file_type, FileType::CharDevice);
        assert_eq!(get("dev/console").rdev, (5, 1));
        assert_eq!(get("dev/sda").file_type, FileType::BlockDevice);
        assert_eq!(get("dev/sda").rdev, (8, 0));
        assert_eq!(get("etc/big.bin").data, vec![7u8; 70_001]);
        assert!(get("etc/empty").data.is_empty());
        let inos: Vec<_> = entries.iter().map(|entry| entry.ino).collect();
        assert_eq!(inos, (1..=9).collect::<Vec<_>>());
    }
}

#[test]
fn hard_link_group_uses_total_link_count() {
    let mut tree = Tree::new();
    tree.add_file("a", Content::bytes(b"data".to_vec()))
        .unwrap();
    tree.add_hard_link("b/c", "a").unwrap();
    tree.add_hard_link("d", "a").unwrap();
    tree.set_metadata("d", SetMetadata::new().with_uid(9))
        .unwrap();
    let entries = read_all(&archive(&tree, Format::Newc)).unwrap();
    let links: Vec<_> = entries
        .iter()
        .filter(|entry| entry.file_type == FileType::File)
        .map(|entry| {
            (
                entry.name.as_str(),
                entry.ino,
                entry.nlink,
                entry.uid,
                entry.data.len(),
            )
        })
        .collect();
    assert_eq!(
        links,
        [("a", 1, 3, 9, 0), ("b/c", 1, 3, 9, 0), ("d", 1, 3, 9, 4)]
    );
    assert_eq!(entries[1].name, "b");
    assert_eq!(entries[1].ino, 2);
}

#[test]
fn streaming_appends_every_kind() {
    let mut writer = CpioWriter::new(StdIo::new(Vec::new()), &CpioOptions::default());
    let meta = SetMetadata::new();
    writer.append("pipe", &meta, NewEntry::Fifo).unwrap();
    writer.append("sock", &meta, NewEntry::Socket).unwrap();
    let content = Content::bytes(b"x".to_vec());
    writer
        .append_hard_links(&["one", "two"], &meta, &content)
        .unwrap();
    assert_eq!(writer.entries(), 4);
    let bytes = writer.finish().unwrap().into_inner();
    let entries = read_all(&bytes).unwrap();
    assert_eq!(entries[0].file_type, FileType::Fifo);
    assert_eq!(entries[0].mode, 0o010644);
    assert_eq!(entries[1].file_type, FileType::Socket);
    assert_eq!((entries[2].ino, entries[3].ino), (3, 3));
    assert_eq!((entries[2].data.len(), entries[3].data.len()), (0, 1));
}

#[test]
fn writer_rejects_empty_symlink_target() {
    let mut writer = CpioWriter::new(StdIo::new(Vec::new()), &CpioOptions::default());
    let err = writer
        .append("link", &SetMetadata::new(), NewEntry::Symlink(b""))
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::InvalidInput, Some(Detail::Entry))
    );
    assert_eq!(writer.bytes_written(), 0);
}

#[test]
fn writer_rejects_bad_names_and_fields_before_writing() {
    let mut writer = CpioWriter::new(StdIo::new(Vec::new()), &CpioOptions::default());
    let meta = SetMetadata::new();
    for (name, kind) in [
        ("", ErrorKind::InvalidInput),
        ("TRAILER!!!", ErrorKind::InvalidInput),
        (&"a".repeat(4096)[..], ErrorKind::NameTooLong),
    ] {
        assert_eq!(
            writer
                .append(name, &meta, NewEntry::Dir)
                .unwrap_err()
                .kind(),
            kind
        );
    }
    let old = SetMetadata::new()
        .with_times(FileTimes::new().with_modified(DateTime::from_unix_seconds(-1).unwrap()));
    assert_eq!(
        writer
            .append("old", &old, NewEntry::Dir)
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(writer.bytes_written(), 0);

    let mut odc = CpioWriter::new(
        StdIo::new(Vec::new()),
        &CpioOptions::default().with_format(Format::Odc),
    );
    let big_uid = SetMetadata::new().with_uid(1 << 18);
    assert_eq!(
        odc.append("x", &big_uid, NewEntry::Dir).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        odc.append(
            "d",
            &meta,
            NewEntry::Device(DeviceKind::Char, DeviceNumber::new(1, 300))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::LimitExceeded
    );
    let mut binary = CpioWriter::new(
        StdIo::new(Vec::new()),
        &CpioOptions::default().with_format(Format::Binary),
    );
    assert_eq!(
        binary.append("x", &meta, NewEntry::Dir).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn dropped_metadata_is_reported() {
    let mut tree = Tree::new();
    tree.add_file("f", Content::empty()).unwrap();
    let time = DateTime::new(5, 500).unwrap();
    tree.set_metadata(
        "f",
        SetMetadata::new().with_times(FileTimes::new().with_modified(time).with_accessed(time)),
    )
    .unwrap();
    let mut out = StdIo::new(Vec::new());
    let report = hadris_cpio::sync::write(&mut out, &tree, &CpioOptions::default()).unwrap();
    assert_eq!(report.entries(), 1);
    assert_eq!(report.warnings().len(), 1);
    assert_eq!(report.warnings()[0].path(), "f");
    assert_eq!(report.warnings()[0].kind(), WarningKind::IgnoredMetadata);
    assert!(report.warnings()[0].message().contains("access time"));
}

#[test]
fn crc_reader_rejects_corrupt_data() {
    let mut bytes = archive(&sample_tree(), Format::NewcCrc);
    let at = bytes
        .windows(7)
        .position(|window| window == b"busybox")
        .unwrap();
    bytes[at] ^= 1;
    let err = read_all(&bytes).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::Corrupt, Some(Detail::Checksum))
    );

    let mut reader = CpioReader::new(Cursor::new(&bytes));
    let err = loop {
        match reader.next_entry() {
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the skipped data must be checked"),
            Err(err) => break err,
        }
    };
    assert_eq!(err.detail(), Some(Detail::Checksum));
    assert!(reader.next_entry().unwrap().is_none());
}

#[test]
fn reader_rejects_non_nul_filename_terminator() {
    let mut bytes = newc_entry(b"ab", 0o100644, b"", None);
    bytes[112] = b'c';
    assert_eq!(read_all(&bytes).unwrap_err().detail(), Some(Detail::Name));

    let mut garbage = newc_entry(b"a\0b", 0o100644, b"", None);
    garbage[110..114].copy_from_slice(b"a\0b\0");
    assert_eq!(read_all(&garbage).unwrap_err().detail(), Some(Detail::Name));
    assert_eq!(
        read_all(&newc_entry(b"", 0o100644, b"", None))
            .unwrap_err()
            .detail(),
        Some(Detail::Name)
    );
}

#[test]
fn alignment_padding_edge_cases() {
    for len in 0..8 {
        let name = "n".repeat(len + 1);
        let data = vec![0xAB; len];
        let mut bytes = newc_entry(name.as_bytes(), 0o100644, &data, None);
        assert_eq!(bytes.len() % 4, 0);
        bytes.extend(trailer());
        let entries = read_all(&bytes).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, name);
        assert_eq!(entries[0].data, data);
    }
}

#[test]
fn reader_rejects_nonzero_name_and_data_padding() {
    let mut name_pad = newc_entry(b"xy", 0o100644, b"abc", None);
    name_pad[113] = 1;
    assert_eq!(
        read_all(&name_pad).unwrap_err().detail(),
        Some(Detail::Padding)
    );
    let mut data_pad = newc_entry(b"x", 0o100644, b"abc", None);
    let last = data_pad.len() - 1;
    data_pad[last] = 1;
    assert_eq!(
        read_all(&data_pad).unwrap_err().detail(),
        Some(Detail::Padding)
    );
}

#[test]
fn trailer_detection() {
    let mut bytes = newc_entry(b"a", 0o100644, b"1", None);
    bytes.extend(trailer());
    bytes.extend(newc_entry(b"hidden", 0o100644, b"2", None));
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    assert!(reader.next_entry().unwrap().is_some());
    assert!(reader.next_entry().unwrap().is_none());
    assert!(reader.at_trailer());
    assert!(reader.next_entry().unwrap().is_none());
}

#[test]
fn reader_rejects_trailer_with_nonzero_filesize() {
    let bytes = newc_entry(b"TRAILER!!!", 0, b"x", None);
    assert_eq!(
        read_all(&bytes).unwrap_err().detail(),
        Some(Detail::Trailer)
    );
}

#[test]
fn reader_accepts_aligned_eof_without_trailer() {
    let bytes = newc_entry(b"a", 0o100644, b"12345", None);
    assert_eq!(read_all(&bytes).unwrap().len(), 1);
    let strict = ReaderOptions::new().with_strict_trailer();
    let err = read_all_with(&bytes, strict).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::Corrupt, Some(Detail::Trailer))
    );

    let mut cut = bytes.clone();
    cut.extend_from_slice(&trailer()[..60]);
    assert_eq!(
        read_all(&cut).unwrap_err().detail(),
        Some(Detail::Truncated)
    );
    let data_cut = &bytes[..bytes.len() - 5];
    assert_eq!(
        read_all(data_cut).unwrap_err().detail(),
        Some(Detail::Truncated)
    );
}

#[test]
fn concatenated_archives_read_on_request() {
    let mut first = newc_entry(b"microcode", 0o100644, b"ucode", None);
    first.extend(trailer());
    first.resize(512, 0);
    first.extend(newc_entry(b"init", 0o100755, b"sh", None));
    first.extend(trailer());
    let mut reader = CpioReader::new(Cursor::new(&first));
    let mut names = Vec::new();
    loop {
        while let Some(entry) = reader.next_entry().unwrap() {
            names.push(entry.name_str().unwrap().to_string());
        }
        if !reader.at_trailer() {
            break;
        }
        reader.continue_after_trailer();
    }
    assert_eq!(names, ["microcode", "init"]);
}

#[test]
fn malformed_headers_are_corrupt() {
    let mut magic = newc_entry(b"a", 0o100644, b"", None);
    magic[5] = b'9';
    assert_eq!(read_all(&magic).unwrap_err().detail(), Some(Detail::Magic));
    let mut hex = newc_entry(b"a", 0o100644, b"", None);
    assert_eq!(
        read_all(&hex[..50]).unwrap_err().detail(),
        Some(Detail::Truncated)
    );
    hex[20] = b'z';
    assert_eq!(read_all(&hex).unwrap_err().detail(), Some(Detail::Field));
    let mut check = newc_entry(b"a", 0o100644, b"", None);
    check[109] = b'1';
    assert_eq!(read_all(&check).unwrap_err().detail(), Some(Detail::Check));
    let mut kind = newc_entry(b"a", 0o100644, b"", None);
    kind[14..22].copy_from_slice(b"000F01A4");
    assert_eq!(read_all(&kind).unwrap_err().detail(), Some(Detail::Field));
    let mut name = newc_entry(b"a", 0o100644, b"", None);
    name[94..102].copy_from_slice(b"00001001");
    assert_eq!(read_all(&name).unwrap_err().detail(), Some(Detail::Name));
}

#[test]
fn huge_claimed_sizes_end_with_an_error() {
    let mut bytes = newc_entry(b"x", 0o100644, b"", None);
    bytes[54..62].copy_from_slice(b"FFFFFFFF");
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    let entry = reader.next_entry().unwrap().unwrap();
    assert_eq!(entry.len(), u64::from(u32::MAX));
    let _ = entry;
    assert_eq!(
        reader.next_entry().unwrap_err().detail(),
        Some(Detail::Truncated)
    );
}

#[test]
fn odc_and_binary_archives_read() {
    let odc = archive(&sample_tree(), Format::Odc);
    assert_eq!(&odc[..6], b"070707");
    let mut reader = CpioReader::new(Cursor::new(&odc));
    assert_eq!(reader.next_entry().unwrap().unwrap().format(), Format::Odc);

    let mut binary = Vec::new();
    for little in [true, false] {
        let word = |value: u16| {
            if little {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            }
        };
        for (name, mode, data) in [
            (&b"hi\0"[..], 0o100644u16, &b"abc"[..]),
            (b"TRAILER!!!\0", 0, b""),
        ] {
            let words = [
                0o070707,
                1,
                2,
                mode,
                0,
                0,
                1,
                0,
                0,
                0,
                name.len() as u16,
                0,
                data.len() as u16,
            ];
            for value in words {
                binary.extend_from_slice(&word(value));
            }
            binary.extend_from_slice(name);
            if name.len() % 2 == 1 {
                binary.push(0);
            }
            binary.extend_from_slice(data);
            if data.len() % 2 == 1 {
                binary.push(0);
            }
        }
        let entries = read_all(&binary).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].name.as_str(), &entries[0].data[..]),
            ("hi", &b"abc"[..])
        );
        binary.clear();
    }
}

#[test]
fn metadata_of_an_entry() {
    let bytes = archive(&sample_tree(), Format::Newc);
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    while let Some(entry) = reader.next_entry().unwrap() {
        if entry.name() == b"init" {
            let meta = entry.metadata();
            assert_eq!(meta.file_type(), FileType::File);
            assert_eq!(meta.len(), 10);
            assert_eq!(meta.permissions(), Some(Mode::new(0o755)));
            assert_eq!(meta.owner(), Some((1000, 100)));
            assert_eq!(
                meta.times().modified(),
                Some(DateTime::from_unix_seconds(1_700_000_000).unwrap())
            );
        }
    }
}

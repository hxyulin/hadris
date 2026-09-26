use hadris_io::sync::Read;
mod common;

use common::{archive, newc_entry, read_all, read_all_with, trailer};
use hadris_cpio::sync::{CpioReader, Writer, read_tree};
use hadris_cpio::{CpioOptions, Detail, Format, ReaderOptions};
use hadris_fs::{
    Content, DateTime, DeviceNumber, ErrorKind, Extent, Field, FileType, Node, Owner, Permissions,
    SetAttr, Tree, WarningKind,
};
use hadris_io::sync::Write;
use hadris_io::{Cursor, StdIo};

fn sample_tree() -> Tree {
    let mut tree = Tree::new();
    let attrs = SetAttr::new()
        .with_permissions(Permissions::new(0o755))
        .with_owner(Owner::new(1000, 100))
        .with_modified(DateTime::from_unix_seconds(1_700_000_000).unwrap());
    tree.insert(
        "init",
        Node::file(Content::bytes(b"#!/bin/sh\n".to_vec())).with_attrs(attrs),
    )
    .unwrap();
    tree.insert("etc/empty", Node::file(Content::empty()))
        .unwrap();
    tree.insert("etc/big.bin", Node::file(Content::bytes(vec![7u8; 70_001])))
        .unwrap();
    tree.insert("dev", Node::dir()).unwrap();
    tree.insert(
        "dev/console",
        Node::special(FileType::CharDevice, Some(DeviceNumber::new(5, 1))),
    )
    .unwrap();
    tree.insert(
        "dev/sda",
        Node::special(FileType::BlockDevice, Some(DeviceNumber::new(8, 0))),
    )
    .unwrap();
    tree.insert("bin/sh", Node::symlink("busybox")).unwrap();
    tree
}

#[test]
fn every_format_reads_back() {
    for format in [Format::Newc, Format::Crc, Format::Odc] {
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
fn the_plan_is_the_report_and_locates_data() {
    let tree = sample_tree();
    for format in [Format::Newc, Format::Crc, Format::Odc] {
        let options = CpioOptions::new().with_format(format);
        let plan = hadris_cpio::plan(&tree, &options).unwrap();
        let mut out = StdIo::new(Vec::new());
        let report = hadris_cpio::sync::write(&mut out, &tree, &options).unwrap();
        let bytes = out.into_inner();
        assert_eq!(plan, report);
        assert_eq!(report.size(), bytes.len() as u64);
        let [extent] = report.extents("/init").unwrap() else {
            panic!("one extent")
        };
        let at = extent.offset() as usize;
        assert_eq!(&bytes[at..at + extent.len() as usize], b"#!/bin/sh\n");
        assert_eq!(report.extents("etc/empty").unwrap()[0].len(), 0);
        assert!(report.extents("bin/sh").is_none());
        assert!(report.warnings().is_empty());
    }
}

#[test]
fn hard_link_group_uses_total_link_count() {
    let mut tree = Tree::new();
    let attrs = SetAttr::new().with_owner(Owner::new(9, 0));
    tree.insert(
        "a",
        Node::file(Content::bytes(b"data".to_vec())).with_attrs(attrs),
    )
    .unwrap();
    tree.link("a", "b/c").unwrap();
    tree.link("a", "d").unwrap();
    let bytes = archive(&tree, Format::Newc);
    let entries = read_all(&bytes).unwrap();
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

    let report = hadris_cpio::plan(&tree, &CpioOptions::new()).unwrap();
    let extent = report.extents("d").unwrap()[0];
    assert_eq!(report.extents("a").unwrap(), [extent]);
    assert_eq!(report.extents("b/c").unwrap(), [extent]);
    let at = extent.offset() as usize;
    assert_eq!(&bytes[at..at + 4], b"data");
}

#[test]
fn streaming_appends_every_kind() {
    let mut writer = Writer::new(StdIo::new(Vec::new()), &CpioOptions::default());
    writer
        .append("pipe", &Node::special(FileType::Fifo, None))
        .unwrap();
    writer
        .append("sock", &Node::special(FileType::Socket, None))
        .unwrap();
    let file = Node::file(Content::bytes(b"x".to_vec()));
    writer.append_hard_links(&["one", "two"], &file).unwrap();
    let mut entry = writer.append_file("made", &SetAttr::new(), 5).unwrap();
    entry.write_all(b"he").unwrap();
    assert_eq!(
        entry.write(b"llo!").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    entry.write_all(b"llo").unwrap();
    entry.finish().unwrap();
    assert_eq!(writer.entries(), 5);
    let (out, report) = writer.finish().unwrap();
    let bytes = out.into_inner();
    assert_eq!(report.size(), bytes.len() as u64);
    let made = report.extents("made").unwrap()[0];
    assert_eq!(made, Extent::new(made.offset(), 5));
    assert_eq!(&bytes[made.offset() as usize..][..5], b"hello");
    assert_eq!(report.extents("one"), report.extents("two"));
    let entries = read_all(&bytes).unwrap();
    assert_eq!(entries[0].file_type, FileType::Fifo);
    assert_eq!(entries[0].mode, 0o010644);
    assert_eq!(entries[1].file_type, FileType::Socket);
    assert_eq!((entries[2].ino, entries[3].ino), (3, 3));
    assert_eq!((entries[2].data.len(), entries[3].data.len()), (0, 1));
    assert_eq!(
        (entries[4].ino, entries[4].data.as_slice()),
        (4, &b"hello"[..])
    );
}

#[test]
fn an_unfinished_entry_blocks_the_writer() {
    let mut writer = Writer::new(StdIo::new(Vec::new()), &CpioOptions::default());
    let mut entry = writer.append_file("short", &SetAttr::new(), 3).unwrap();
    entry.write_all(b"ab").unwrap();
    let err = entry.finish().unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::InvalidInput, Some(&b"short"[..]))
    );
    assert_eq!(
        writer.append("x", &Node::dir()).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(writer.finish().unwrap_err().kind(), ErrorKind::InvalidInput);

    let mut crc = Writer::new(
        StdIo::new(Vec::new()),
        &CpioOptions::new().with_format(Format::Crc),
    );
    assert_eq!(
        crc.append_file("f", &SetAttr::new(), 1).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn writer_rejects_empty_symlink_target() {
    let mut writer = Writer::new(StdIo::new(Vec::new()), &CpioOptions::default());
    let err = writer.append("link", &Node::symlink("")).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail().and_then(Detail::from_code)),
        (ErrorKind::InvalidInput, Some(Detail::Entry))
    );
    assert_eq!(writer.bytes_written(), 0);
}

#[test]
fn writer_rejects_bad_names_and_fields_before_writing() {
    let mut writer = Writer::new(StdIo::new(Vec::new()), &CpioOptions::default());
    for (name, kind) in [
        ("", ErrorKind::InvalidInput),
        ("TRAILER!!!", ErrorKind::InvalidInput),
        (&"a".repeat(4096)[..], ErrorKind::NameTooLong),
    ] {
        let err = writer.append(name, &Node::dir()).unwrap_err();
        assert_eq!((err.kind(), err.path()), (kind, Some(name.as_bytes())));
    }
    let old = SetAttr::new().with_modified(DateTime::from_unix_seconds(-1).unwrap());
    assert_eq!(
        writer
            .append("old", &Node::dir().with_attrs(old))
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(writer.bytes_written(), 0);

    let mut odc = Writer::new(
        StdIo::new(Vec::new()),
        &CpioOptions::default().with_format(Format::Odc),
    );
    let big_uid = SetAttr::new().with_owner(Owner::new(1 << 18, 0));
    assert_eq!(
        odc.append("x", &Node::dir().with_attrs(big_uid))
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    let device = Node::special(FileType::CharDevice, Some(DeviceNumber::new(1, 300)));
    assert_eq!(
        odc.append("d", &device).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let mut binary = Writer::new(
        StdIo::new(Vec::new()),
        &CpioOptions::default().with_format(Format::Binary),
    );
    assert_eq!(
        binary.append("x", &Node::dir()).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn files_over_the_format_limit_are_too_large() {
    let mut tree = Tree::new();
    let huge = Content::stored([Extent::new(0, u64::from(u32::MAX) + 1)]);
    tree.insert("big", Node::file(huge)).unwrap();
    let err = hadris_cpio::plan(&tree, &CpioOptions::default()).unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::FileTooLarge, Some(&b"big"[..]))
    );

    let odc = CpioOptions::default().with_format(Format::Odc);
    assert!(hadris_cpio::plan(&tree, &odc).is_ok());
    let mut tree = Tree::new();
    let huge = Content::stored([Extent::new(0, Format::Odc.max_file_size() + 1)]);
    tree.insert("big", Node::file(huge)).unwrap();
    assert_eq!(
        hadris_cpio::plan(&tree, &odc).unwrap_err().kind(),
        ErrorKind::FileTooLarge
    );
    assert_eq!(Format::Newc.max_file_size(), u64::from(u32::MAX));
}

#[test]
fn unreadable_content_fails_before_writing() {
    let mut tree = Tree::new();
    tree.insert("a", Node::file(Content::bytes("a"))).unwrap();
    tree.insert("b", Node::file(Content::stored([Extent::new(0, 4)])))
        .unwrap();
    let mut out = StdIo::new(Vec::new());
    let err = hadris_cpio::sync::write(&mut out, &tree, &CpioOptions::new()).unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::Unsupported, Some(&b"b"[..]))
    );
    assert!(out.into_inner().is_empty());
}

#[test]
fn dropped_metadata_is_reported() {
    let mut tree = Tree::new();
    let time = DateTime::new(5, 500).unwrap();
    let attrs = SetAttr::new().with_modified(time).with_accessed(time);
    tree.insert("f", Node::file(Content::empty()).with_attrs(attrs))
        .unwrap();
    tree.insert("g", Node::file(Content::empty()).with_attrs(attrs))
        .unwrap();
    let mut out = StdIo::new(Vec::new());
    let report = hadris_cpio::sync::write(&mut out, &tree, &CpioOptions::default()).unwrap();
    let warnings: Vec<_> = report
        .warnings()
        .iter()
        .map(|warning| (warning.kind(), warning.count(), warning.path()))
        .collect();
    assert_eq!(
        warnings,
        [
            (WarningKind::Dropped(Field::Accessed), 2, None),
            (WarningKind::Dropped(Field::Modified), 2, None)
        ]
    );
}

#[test]
fn the_options_time_fills_unset_modification_times() {
    let mut tree = Tree::new();
    tree.insert("a", Node::file(Content::empty())).unwrap();
    let set = SetAttr::new().with_modified(DateTime::from_unix_seconds(7).unwrap());
    tree.insert("b", Node::dir().with_attrs(set)).unwrap();
    let options = CpioOptions::new().with_time(DateTime::from_unix_seconds(1_000).unwrap());
    let mut out = StdIo::new(Vec::new());
    hadris_cpio::sync::write(&mut out, &tree, &options).unwrap();
    let mtimes: Vec<_> = read_all(&out.into_inner())
        .unwrap()
        .iter()
        .map(|entry| entry.mtime)
        .collect();
    assert_eq!(mtimes, [1_000, 7]);
}

#[test]
fn read_tree_restores_what_write_wrote() {
    let mut tree = sample_tree();
    tree.link("init", "sbin/init").unwrap();
    tree.link("init", "linuxrc").unwrap();
    let bytes = archive(&tree, Format::Newc);
    let back = read_tree(&mut CpioReader::new(Cursor::new(&bytes))).unwrap();
    assert_eq!(archive(&back, Format::Newc), bytes);
    let init = back.entry("sbin/init").unwrap();
    assert_eq!(init.links(), 3);
    assert_eq!(
        init.node().content().unwrap().as_bytes(),
        Some(&b"#!/bin/sh\n"[..])
    );
    assert_eq!(
        back.get("dev/console").unwrap().device(),
        Some(DeviceNumber::new(5, 1))
    );
}

#[test]
fn read_tree_takes_relative_paths_and_refuses_parents() {
    let mut archive = newc_entry(b".", 0o040700, b"", None);
    archive.extend(newc_entry(b"./a", 0o100644, b"x", None));
    archive.extend(newc_entry(b"/b/c", 0o100644, b"y", None));
    archive.extend(trailer());
    let tree = read_tree(&mut CpioReader::new(Cursor::new(&archive))).unwrap();
    assert_eq!(
        tree.root().node().attrs().permissions(),
        Some(Permissions::new(0o700))
    );
    assert_eq!(
        tree.get("a").unwrap().content().unwrap().as_bytes(),
        Some(&b"x"[..])
    );
    assert_eq!(
        tree.get("b/c").unwrap().content().unwrap().as_bytes(),
        Some(&b"y"[..])
    );

    let mut archive = newc_entry(b"a/../../x", 0o100644, b"", None);
    archive.extend(trailer());
    let err = read_tree(&mut CpioReader::new(Cursor::new(&archive))).unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::InvalidInput, Some(&b"a/../../x"[..]))
    );
}

#[test]
fn crc_reader_rejects_corrupt_data() {
    let mut bytes = archive(&sample_tree(), Format::Crc);
    let at = bytes
        .windows(7)
        .position(|window| window == b"busybox")
        .unwrap();
    bytes[at] ^= 1;
    let err = read_all(&bytes).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail().and_then(Detail::from_code)),
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
    assert_eq!(
        err.detail().and_then(Detail::from_code),
        Some(Detail::Checksum)
    );
    assert!(reader.next_entry().unwrap().is_none());
}

#[test]
fn reader_rejects_non_nul_filename_terminator() {
    let mut bytes = newc_entry(b"ab", 0o100644, b"", None);
    bytes[112] = b'c';
    assert_eq!(
        read_all(&bytes)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Name)
    );

    let mut garbage = newc_entry(b"a\0b", 0o100644, b"", None);
    garbage[110..114].copy_from_slice(b"a\0b\0");
    assert_eq!(
        read_all(&garbage)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Name)
    );
    assert_eq!(
        read_all(&newc_entry(b"", 0o100644, b"", None))
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Name)
    );
}

#[test]
fn names_that_are_not_utf8_stay_bytes() {
    let bytes = newc_entry(b"caf\xE9", 0o100644, b"", None);
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    let entry = reader.next_entry().unwrap().unwrap();
    assert_eq!(entry.path(), b"caf\xE9");
    assert!(entry.path_str().is_err());
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
        read_all(&name_pad)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Padding)
    );
    let mut data_pad = newc_entry(b"x", 0o100644, b"abc", None);
    let last = data_pad.len() - 1;
    data_pad[last] = 1;
    assert_eq!(
        read_all(&data_pad)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
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
    assert!(reader.next_entry().unwrap().is_none());
}

#[test]
fn reader_rejects_trailer_with_nonzero_filesize() {
    let bytes = newc_entry(b"TRAILER!!!", 0, b"x", None);
    assert_eq!(
        read_all(&bytes)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
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
        (err.kind(), err.detail().and_then(Detail::from_code)),
        (ErrorKind::Corrupt, Some(Detail::Trailer))
    );

    let mut cut = bytes.clone();
    cut.extend_from_slice(&trailer()[..60]);
    assert_eq!(
        read_all(&cut)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Truncated)
    );
    let data_cut = &bytes[..bytes.len() - 5];
    assert_eq!(
        read_all(data_cut)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
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
            names.push(entry.path_str().unwrap().to_string());
        }
        if !reader.next_segment().unwrap() {
            break;
        }
    }
    assert_eq!(names, ["microcode", "init"]);
}

#[test]
fn malformed_headers_are_corrupt() {
    let mut magic = newc_entry(b"a", 0o100644, b"", None);
    magic[5] = b'9';
    let err = read_all(&magic).unwrap_err();
    assert_eq!(
        (err.kind(), Detail::of(&err)),
        (ErrorKind::NotRecognized, Some(Detail::Magic))
    );
    let mut second = newc_entry(b"a", 0o100644, b"", None);
    second.extend_from_slice(&magic);
    let err = read_all(&second).unwrap_err();
    assert_eq!(
        (err.kind(), Detail::of(&err)),
        (ErrorKind::Corrupt, Some(Detail::Magic))
    );
    let mut hex = newc_entry(b"a", 0o100644, b"", None);
    assert_eq!(
        read_all(&hex[..50])
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Truncated)
    );
    hex[20] = b'z';
    assert_eq!(
        read_all(&hex)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Field)
    );
    let mut check = newc_entry(b"a", 0o100644, b"", None);
    check[109] = b'1';
    assert_eq!(
        read_all(&check)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Check)
    );
    let mut kind = newc_entry(b"a", 0o100644, b"", None);
    kind[14..22].copy_from_slice(b"000F01A4");
    assert_eq!(
        read_all(&kind)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Field)
    );
    let mut name = newc_entry(b"a", 0o100644, b"", None);
    name[94..102].copy_from_slice(b"00001001");
    assert_eq!(
        read_all(&name)
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Name)
    );
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
        reader
            .next_entry()
            .unwrap_err()
            .detail()
            .and_then(Detail::from_code),
        Some(Detail::Truncated)
    );
}

#[test]
fn odc_and_binary_archives_read() {
    let odc = archive(&sample_tree(), Format::Odc);
    assert_eq!(&odc[..6], b"070707");
    let mut reader = CpioReader::new(Cursor::new(&odc));
    let entry = reader.next_entry().unwrap().unwrap();
    assert_eq!(entry.format(), Format::Odc);
    assert_eq!(entry.offset(), 0);
    assert_eq!(entry.data_offset(), 76 + entry.path().len() as u64 + 1);

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
        let mut reader = CpioReader::new(Cursor::new(&binary));
        let entry = reader.next_entry().unwrap().unwrap();
        assert_eq!((entry.offset(), entry.data_offset()), (0, 30));
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
        if entry.path() == b"init" {
            let meta = entry.metadata();
            assert_eq!(meta.file_type(), FileType::File);
            assert_eq!(meta.len(), 10);
            assert_eq!(meta.permissions(), Permissions::new(0o755));
            assert_eq!(meta.owner(), Some(Owner::new(1000, 100)));
            assert_eq!(
                meta.modified(),
                Some(DateTime::from_unix_seconds(1_700_000_000).unwrap())
            );
        }
    }
}

#[test]
fn caller_buffer_limits_include_nul_and_preserve_non_utf8_paths() {
    let bytes = newc_entry(b"a\xff", 0o100644, b"data", None);
    let mut storage = [0u8; 3];
    let mut reader =
        CpioReader::with_buffer(Cursor::new(&bytes), &mut storage[..], ReaderOptions::new());
    let mut entry = reader.next_entry().unwrap().unwrap();
    assert_eq!(entry.path(), b"a\xff");
    assert!(entry.path_str().is_err());
    assert_eq!((entry.offset(), entry.data_offset()), (0, 116));
    let mut data = [0; 4];
    entry.read_exact(&mut data).unwrap();
    assert_eq!(&data, b"data");
    assert_eq!(entry.data_offset(), 116);
    assert!(reader.next_entry().unwrap().is_none());
    for capacity in [0, 1, 2] {
        let mut reader =
            CpioReader::with_buffer(Cursor::new(&bytes), vec![0; capacity], ReaderOptions::new());
        assert_eq!(
            reader.next_entry().unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
        assert!(reader.next_entry().unwrap().is_none());
    }
}

#[test]
fn caller_buffer_accepts_names_beyond_default_limit() {
    let path = vec![b'x'; hadris_cpio::raw::PATH_MAX + 1];
    let bytes = newc_entry(&path, 0o100644, b"", None);
    let mut default = CpioReader::new(Cursor::new(&bytes));
    assert_eq!(
        default.next_entry().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let mut reader = CpioReader::with_buffer(
        Cursor::new(&bytes),
        vec![0; path.len() + 1],
        ReaderOptions::new(),
    );
    assert_eq!(reader.next_entry().unwrap().unwrap().path(), path);
}

#[test]
fn custom_buffer_still_validates_termination() {
    let mut bytes = newc_entry(b"abc", 0o100644, b"", None);
    bytes[113] = b'x';
    let mut reader = CpioReader::with_buffer(Cursor::new(&bytes), [0; 4], ReaderOptions::new());
    assert_eq!(reader.next_entry().unwrap_err().kind(), ErrorKind::Corrupt);
}

#[test]
fn segment_state_offsets_and_pending_byte_recovery() {
    let mut bytes = trailer();
    bytes.resize(512, 0);
    let second = newc_entry(b"x", 0o100644, b"abc", None);
    bytes.extend(&second);
    bytes.extend(trailer());
    bytes.extend([0; 16]);
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    assert_eq!(
        reader.next_segment().unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert!(reader.next_entry().unwrap().is_none());
    assert!(reader.next_segment().unwrap());
    assert_eq!(reader.offset(), 513);
    assert_eq!(
        reader.next_segment().unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let (mut input, buffer, pending) = reader.into_parts();
    assert_eq!(buffer.len(), hadris_cpio::raw::PATH_MAX);
    assert_eq!(pending, Some(b'0'));
    let mut next = [0];
    input.read_exact(&mut next).unwrap();
    assert_eq!(next[0], second[1]);

    let mut reader = CpioReader::new(Cursor::new(&bytes));
    assert!(reader.next_entry().unwrap().is_none());
    assert!(reader.next_segment().unwrap());
    let entry = reader.next_entry().unwrap().unwrap();
    assert_eq!((entry.offset(), entry.data_offset()), (512, 624));
    assert_eq!(entry.path(), b"x");
    assert!(reader.next_entry().unwrap().is_none());
    assert!(!reader.next_segment().unwrap());
    assert!(!reader.next_segment().unwrap());
    assert_eq!(reader.offset(), bytes.len() as u64);
}

#[test]
fn segment_probe_does_not_claim_header_validity() {
    let mut bytes = trailer();
    bytes.extend(b"garbage");
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    assert!(reader.next_entry().unwrap().is_none());
    assert!(reader.next_segment().unwrap());
    assert_eq!(reader.next_entry().unwrap_err().kind(), ErrorKind::Corrupt);
    assert!(!reader.next_segment().unwrap());
}

#[test]
fn crc_trailer_checksum_is_verified() {
    let bytes = newc_entry(b"TRAILER!!!", 0, b"", Some(1));
    let mut reader = CpioReader::new(Cursor::new(&bytes));
    let error = reader.next_entry().unwrap_err();
    assert_eq!(Detail::of(&error), Some(Detail::Checksum));
}

#[test]
fn caller_buffer_must_fit_trailer_and_both_slice_views() {
    let mut bytes = newc_entry(b"x", 0o100644, b"", None);
    bytes.extend(trailer());
    let mut reader = CpioReader::with_buffer(Cursor::new(&bytes), [0; 2], ReaderOptions::new());
    assert_eq!(reader.next_entry().unwrap().unwrap().path(), b"x");
    assert_eq!(
        reader.next_entry().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    struct UnequalViews([u8; 16]);
    impl AsRef<[u8]> for UnequalViews {
        fn as_ref(&self) -> &[u8] {
            &self.0[..1]
        }
    }
    impl AsMut<[u8]> for UnequalViews {
        fn as_mut(&mut self) -> &mut [u8] {
            &mut self.0
        }
    }
    let mut reader = CpioReader::with_buffer(
        Cursor::new(&bytes),
        UnequalViews([0; 16]),
        ReaderOptions::new(),
    );
    assert_eq!(
        reader.next_entry().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

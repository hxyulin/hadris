//! Security/robustness audit POCs for hadris-cpio (reader paths the fuzzer
//! does not reach: `next_entry_with_buf`, `read_entry_data`, `skip_entry_data`).

use hadris_cpio::read::CpioArchiveReader;

/// Build a newc archive with a single regular file entry (no trailer).
/// `name` must not contain NUL; `data_len_on_disk` bytes of `0x41` are appended
/// as file data (may differ from the claimed filesize).
fn single_file_archive(claimed_filesize: u32, data_len_on_disk: usize) -> Vec<u8> {
    let name = b"x";
    let namesize = (name.len() + 1) as u32;
    let mut h = Vec::new();
    h.extend_from_slice(b"070701");
    let mut field = |v: u32| h.extend_from_slice(format!("{v:08X}").as_bytes());
    field(1); // ino
    field(0o100644); // mode: regular file
    field(0); // uid
    field(0); // gid
    field(1); // nlink
    field(0); // mtime
    field(claimed_filesize); // filesize
    field(0); // devmajor
    field(0); // devminor
    field(0); // rdevmajor
    field(0); // rdevminor
    field(namesize); // namesize
    field(0); // check
    assert_eq!(h.len(), 110);
    h.extend_from_slice(name);
    h.push(0);
    let name_pad = (4 - ((110 + namesize as usize) % 4)) % 4;
    h.extend_from_slice(&[0u8; 3][..name_pad]);
    h.extend_from_slice(&vec![0x41u8; data_len_on_disk]);
    h
}

/// Regression: `read_entry_data` must return `Err(Error::BufferSizeMismatch)`
/// when the attacker-controlled on-disk `filesize` does not match the caller's
/// buffer length, not panic (previously an `assert_eq!`, an abort on no_std).
#[test]
fn read_entry_data_rejects_image_controlled_size_mismatch() {
    // Archive claims filesize = 100; caller uses a fixed 16-byte scratch buffer
    // (the natural pattern for the no-alloc API this method belongs to).
    let archive = single_file_archive(100, 100);
    let mut reader = CpioArchiveReader::new(archive.as_slice());
    let mut name_buf = [0u8; 16];
    let entry = reader
        .next_entry_with_buf(&mut name_buf)
        .expect("header parses")
        .expect("not trailer");
    assert_eq!(entry.file_size(), 100);
    let mut data_buf = [0u8; 16];
    let err = reader
        .read_entry_data(&entry, &mut data_buf)
        .expect_err("must reject mismatched buffer");
    assert!(matches!(
        err,
        hadris_cpio::Error::BufferSizeMismatch {
            expected: 100,
            actual: 16
        }
    ));
    // No data was consumed; the reader offset still points at the data start.
    let mut data_buf = vec![0u8; 100];
    assert!(reader.read_entry_data(&entry, &mut data_buf).is_ok());
}

/// Disproof: `skip_entry_data` with a bogus ~4 GiB claimed filesize terminates
/// promptly with an I/O error on a short input (no hang, no allocation).
#[test]
fn skip_entry_data_huge_claim_terminates_with_error() {
    let archive = single_file_archive(0xFFFF_FFFF, 16);
    let mut reader = CpioArchiveReader::new(archive.as_slice());
    let mut name_buf = [0u8; 16];
    let entry = reader
        .next_entry_with_buf(&mut name_buf)
        .unwrap()
        .expect("not trailer");
    assert!(reader.skip_entry_data(&entry).is_err());
}

/// Disproof: `next_entry_with_buf` rejects a namesize that does not fit the
/// caller buffer; no out-of-bounds write or panic.
#[test]
fn with_buf_rejects_oversized_namesize() {
    // namesize at header offset 94; PATH_MAX is 4096, use a small caller buffer.
    let mut archive = single_file_archive(0, 0);
    archive[94..102].copy_from_slice(b"00001000"); // namesize = 4096
    archive.extend_from_slice(&vec![b'a'; 4096]);
    let mut reader = CpioArchiveReader::new(archive.as_slice());
    let mut name_buf = [0u8; 16];
    assert!(reader.next_entry_with_buf(&mut name_buf).is_err());
}

/// Disproof: interior NUL followed by garbage in the name field is rejected.
#[cfg(feature = "alloc")]
#[test]
fn interior_nul_with_garbage_rejected() {
    let mut archive = Vec::new();
    archive.extend_from_slice(b"070701");
    let field = |v: u32, h: &mut Vec<u8>| h.extend_from_slice(format!("{v:08X}").as_bytes());
    field(1, &mut archive);
    field(0o100644, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(1, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(0, &mut archive);
    field(5, &mut archive); // namesize = 5
    field(0, &mut archive);
    archive.extend_from_slice(b"a\0bb\0"); // name "a", garbage "bb", NUL
    archive.extend_from_slice(&[0u8; 3]); // pad to 4 (110+5=115 -> pad 1)
    let mut reader = CpioArchiveReader::new(archive.as_slice());
    assert!(reader.next_entry_alloc().is_err());
}

/// Disproof: 070702 checksum mismatch is detected on the skip path too.
#[test]
fn crc_mismatch_detected_on_skip() {
    let mut archive = single_file_archive(4, 4);
    archive[0..6].copy_from_slice(b"070702");
    archive[102..110].copy_from_slice(b"DEADBEEF"); // wrong check
    let mut reader = CpioArchiveReader::new(archive.as_slice());
    let mut name_buf = [0u8; 16];
    let entry = reader
        .next_entry_with_buf(&mut name_buf)
        .unwrap()
        .expect("not trailer");
    assert!(reader.skip_entry_data(&entry).is_err());
}

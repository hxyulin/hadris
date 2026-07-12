//! Integration tests for `hadris_io::Cursor`.
//!
//! These tests exercise the public API of `Cursor` through
//! the `Read` and `Seek` trait implementations.

use hadris_io::{Cursor, Read, ReadExt, Seek, SeekFrom};

#[test]
fn cursor_sequential_reads() {
    let data: Vec<u8> = (0..=255).collect();
    let mut cursor = Cursor::new(&data);

    for i in 0..=255u8 {
        let mut buf = [0u8; 1];
        cursor.read_exact(&mut buf).unwrap();
        assert_eq!(buf[0], i);
    }

    // Should be at end now
    let mut buf = [0u8; 1];
    let result = cursor.read_exact(&mut buf);
    assert!(result.is_err());
}

#[test]
fn cursor_seek_and_read_interleaved() {
    let data = b"ABCDEFGHIJ";
    let mut cursor = Cursor::new(data.as_slice());

    // Read first 3
    let mut buf = [0u8; 3];
    cursor.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ABC");

    // Seek back to start
    cursor.seek(SeekFrom::Start(0)).unwrap();
    cursor.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ABC");

    // Seek to end-3 and read
    cursor.seek(SeekFrom::End(-3)).unwrap();
    cursor.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"HIJ");

    // Seek relative backward
    cursor.seek(SeekFrom::Current(-5)).unwrap();
    cursor.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"FGH");
}

#[test]
fn cursor_read_struct_integration() {
    // Write a known u32 in native byte order and read it back
    let value: u32 = 0xDEAD_BEEF;
    let bytes = value.to_ne_bytes();
    let mut cursor = Cursor::new(&bytes);

    let read_back: u32 = cursor.read_struct().unwrap();
    assert_eq!(read_back, value);
}

#[test]
fn cursor_empty_data() {
    let data = [];
    let mut cursor = Cursor::new(&data);

    assert_eq!(cursor.position(), 0);
    assert_eq!(cursor.get_ref().len(), 0);

    // Read should return 0
    let mut buf = [0u8; 1];
    let n = Read::read(&mut cursor, &mut buf).unwrap();
    assert_eq!(n, 0);

    // read_exact should fail
    let result = cursor.read_exact(&mut buf);
    assert!(result.is_err());
}

#[test]
fn cursor_stream_position() {
    let data = [0u8; 100];
    let mut cursor = Cursor::new(&data);

    assert_eq!(cursor.stream_position().unwrap(), 0);

    cursor.seek(SeekFrom::Start(50)).unwrap();
    assert_eq!(cursor.stream_position().unwrap(), 50);

    cursor.seek_relative(10).unwrap();
    assert_eq!(cursor.stream_position().unwrap(), 60);
}

#[test]
fn cursor_clone_is_independent() {
    let data = [1, 2, 3, 4, 5];
    let mut cursor = Cursor::new(&data);
    cursor.set_position(3);

    let mut clone = cursor.clone();

    // Advance the clone
    let mut buf = [0u8; 1];
    clone.read_exact(&mut buf).unwrap();
    assert_eq!(buf[0], 4);
    assert_eq!(clone.position(), 4);

    // Original should be unchanged
    assert_eq!(cursor.position(), 3);
}

#[test]
fn cursor_seek_past_end_then_read() {
    let data = [1, 2, 3];
    let mut cursor = Cursor::new(&data);

    // Seeking past end is allowed (consistent with std::io::Cursor)
    cursor.seek(SeekFrom::Start(100)).unwrap();
    assert_eq!(cursor.position(), 100);

    // But reading returns 0 bytes (not an error)
    let mut buf = [0u8; 1];
    let n = Read::read(&mut cursor, &mut buf).unwrap();
    assert_eq!(n, 0);
}

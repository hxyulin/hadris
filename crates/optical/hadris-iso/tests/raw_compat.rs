use hadris_iso::raw;

#[test]
fn raw_types_keep_their_identity() {
    let record = raw::DirectoryRecord::new(b"README.TXT;1", &[]).unwrap();
    let standalone: hadris_iso_raw::DirectoryRecord = record;
    assert_eq!(standalone.name(), b"README.TXT;1");
}

#[test]
fn raw_identifier_paths_reject_invalid_utf8() {
    let disk = raw::IsoStr::from_bytes([0xff, b' ']);
    let standalone: hadris_iso_raw::IsoStr<2> = disk;
    assert!(standalone.as_str().is_err());
    assert_eq!(standalone.trimmed(), &[0xff]);
}

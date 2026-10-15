use hadris_iso::raw;

#[test]
fn raw_types_keep_their_identity() {
    let descriptor = raw::VolumeDescriptor::from_bytes([0; 2048]);
    let standalone: hadris_iso_raw::VolumeDescriptor = descriptor;
    assert_eq!(standalone.header().kind(), raw::DescriptorType::BootRecord);
}

#[test]
fn raw_identifier_paths_reject_invalid_utf8() {
    let disk = raw::IsoStr::from_bytes([0xff, b' ']);
    let standalone: hadris_iso_raw::IsoStr<2> = disk;
    assert!(standalone.as_str().is_err());
    assert_eq!(standalone.trimmed(), &[0xff]);
}

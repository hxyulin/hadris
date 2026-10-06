use hadris_cpio::raw;

#[test]
fn raw_types_keep_their_identity() {
    let header = raw::NewcHeader::new(false, &raw::NewcFields::default());
    let standalone: hadris_cpio_raw::NewcHeader = header;
    assert_eq!(standalone.fields(), Some(raw::NewcFields::default()));
}

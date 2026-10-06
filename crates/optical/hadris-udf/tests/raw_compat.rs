use hadris_udf::raw;

#[test]
fn raw_types_keep_their_identity() {
    let tag = raw::Tag::default();
    let standalone: hadris_udf_raw::Tag = tag;
    assert_eq!(standalone.identifier.get(), 0);
}

use crate::{PathTableHeader, U16Be, U16Both, U16Le, U32Be, U32Both, U32Le};

mod boot;
mod dates;
mod records;
mod susp;

fn le32(bytes: &[u8]) -> u32 {
    u32::from(bytes[0])
        | (u32::from(bytes[1]) << 8)
        | (u32::from(bytes[2]) << 16)
        | (u32::from(bytes[3]) << 24)
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from(bytes[3])
        | (u32::from(bytes[2]) << 8)
        | (u32::from(bytes[1]) << 16)
        | (u32::from(bytes[0]) << 24)
}

fn le16(bytes: &[u8]) -> u16 {
    u16::from(bytes[0]) | (u16::from(bytes[1]) << 8)
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from(bytes[1]) | (u16::from(bytes[0]) << 8)
}

#[kani::proof]
fn both_byte_order_fields_match_layout() {
    let raw: [u8; 8] = kani::any();
    let field: U32Both = bytemuck::cast(raw);
    assert_eq!(field.get(), le32(&raw[..4]));
    assert_eq!(field.get_be(), be32(&raw[4..]));
    assert_eq!(field.is_consistent(), le32(&raw[..4]) == be32(&raw[4..]));
    let value: u32 = kani::any();
    let mut written = field;
    written.set(value);
    let bytes: [u8; 8] = bytemuck::cast(written);
    assert_eq!((le32(&bytes[..4]), be32(&bytes[4..])), (value, value));
    assert!(written.is_consistent());
    assert_eq!(written, U32Both::new(value));

    let raw: [u8; 4] = kani::any();
    let field: U16Both = bytemuck::cast(raw);
    assert_eq!(field.get(), le16(&raw[..2]));
    assert_eq!(field.get_be(), be16(&raw[2..]));
    assert_eq!(field.is_consistent(), le16(&raw[..2]) == be16(&raw[2..]));
    let value: u16 = kani::any();
    let mut written = field;
    written.set(value);
    let bytes: [u8; 4] = bytemuck::cast(written);
    assert_eq!((le16(&bytes[..2]), be16(&bytes[2..])), (value, value));
    assert!(written.is_consistent());
    assert_eq!(written, U16Both::new(value));
}

#[kani::proof]
fn single_byte_order_fields_match_layout() {
    let raw: [u8; 4] = kani::any();
    assert_eq!(bytemuck::cast::<_, U32Le>(raw).get(), le32(&raw));
    assert_eq!(bytemuck::cast::<_, U32Be>(raw).get(), be32(&raw));
    assert_eq!(
        bytemuck::cast::<_, U16Le>([raw[0], raw[1]]).get(),
        le16(&raw)
    );
    assert_eq!(
        bytemuck::cast::<_, U16Be>([raw[0], raw[1]]).get(),
        be16(&raw)
    );
    let value: u32 = kani::any();
    let short = value as u16;
    assert_eq!(
        le32(&bytemuck::cast::<_, [u8; 4]>(U32Le::new(value))),
        value
    );
    assert_eq!(
        be32(&bytemuck::cast::<_, [u8; 4]>(U32Be::new(value))),
        value
    );
    assert_eq!(
        le16(&bytemuck::cast::<_, [u8; 2]>(U16Le::new(short))),
        short
    );
    assert_eq!(
        be16(&bytemuck::cast::<_, [u8; 2]>(U16Be::new(short))),
        short
    );
}

#[kani::proof]
fn path_table_records_match_layout() {
    let len: u8 = kani::any();
    let extent: u32 = kani::any();
    let parent: u16 = kani::any();
    let little: [u8; 8] = bytemuck::cast(PathTableHeader::little(len, extent, parent));
    let big: [u8; 8] = bytemuck::cast(PathTableHeader::big(len, extent, parent));
    for bytes in [little, big] {
        assert_eq!((bytes[0], bytes[1]), (len, 0));
    }
    assert_eq!((le32(&little[2..6]), le16(&little[6..])), (extent, parent));
    assert_eq!((be32(&big[2..6]), be16(&big[6..])), (extent, parent));

    let raw: [u8; 8] = kani::any();
    let header: PathTableHeader = bytemuck::cast(raw);
    assert_eq!(header.extent_le(), le32(&raw[2..6]));
    assert_eq!(header.extent_be(), be32(&raw[2..6]));
    assert_eq!(header.parent_le(), le16(&raw[6..]));
    assert_eq!(header.parent_be(), be16(&raw[6..]));
    let identifier = usize::from(raw[0]);
    let padding = identifier % 2;
    assert_eq!(header.record_len(), 8 + identifier + padding);
    assert_eq!(header.record_len() % 2, 0);
}

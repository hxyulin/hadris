use crate::DirectoryRecord;

fn system_use_start(identifier: usize) -> usize {
    if identifier % 2 == 0 {
        34 + identifier
    } else {
        33 + identifier
    }
}

fn both_copies_match(bytes: &[u8], offset: usize, width: usize) -> bool {
    let mut index = 0;
    while index < width {
        if bytes[offset + index] != bytes[offset + 2 * width - 1 - index] {
            return false;
        }
        index += 1;
    }
    true
}

#[kani::proof]
#[kani::unwind(5)]
fn directory_record_parse_matches_specification() {
    let bytes: [u8; 256] = kani::any();
    let available: usize = kani::any();
    kani::assume(available <= bytes.len());
    let result = DirectoryRecord::parse(&bytes[..available]);
    if available == 0 {
        assert!(result.is_err());
        return;
    }
    let len = usize::from(bytes[0]);
    let identifier = usize::from(bytes[32]);
    let valid = len >= 34
        && len % 2 == 0
        && len <= available
        && identifier > 0
        && 33 + identifier <= len
        && bytes[25] & 0x60 == 0
        && both_copies_match(&bytes, 2, 4)
        && both_copies_match(&bytes, 10, 4)
        && both_copies_match(&bytes, 28, 2)
        && (identifier % 2 == 1 || (33 + identifier < len && bytes[33 + identifier] == 0));
    match result {
        Ok(None) => assert_eq!(len, 0),
        Err(_) => assert!(len != 0 && !valid),
        Ok(Some(record)) => {
            assert!(valid);
            assert_eq!(record.len(), len);
            assert_eq!(record.as_bytes().len(), len);
            let index: usize = kani::any();
            kani::assume(index < len);
            assert_eq!(record.as_bytes()[index], bytes[index]);
            assert_eq!(record.name().len(), identifier);
            let index: usize = kani::any();
            kani::assume(index < identifier);
            assert_eq!(record.name()[index], bytes[33 + index]);
            let start = system_use_start(identifier);
            let area = len.saturating_sub(start);
            assert_eq!(record.system_use().len(), area);
            let index: usize = kani::any();
            kani::assume(index < area);
            assert_eq!(record.system_use()[index], bytes[start + index]);
            kani::cover!(
                identifier % 2 == 0 && start < len && len == available,
                "padded identifier with system use filling the input"
            );
        }
    }
}

#[kani::proof]
#[kani::unwind(5)]
fn directory_record_new_parses_back() {
    let name: [u8; 256] = kani::any();
    let system_use: [u8; 256] = kani::any();
    let name_len: usize = kani::any();
    let system_use_len: usize = kani::any();
    kani::assume(name_len <= name.len() && system_use_len <= system_use.len());
    let start = system_use_start(name_len);
    let len = start + system_use_len + system_use_len % 2;
    let made = DirectoryRecord::new(&name[..name_len], &system_use[..system_use_len]);
    let Some(record) = made else {
        assert!(name_len == 0 || len > 255);
        return;
    };
    assert!(name_len > 0 && len <= 255);
    assert_eq!(record.len(), len);
    let parsed = DirectoryRecord::parse(record.as_bytes()).unwrap().unwrap();
    assert_eq!(parsed.len(), len);
    let index: usize = kani::any();
    kani::assume(index < len);
    assert_eq!(parsed.as_bytes()[index], record.as_bytes()[index]);
    assert_eq!(parsed.name().len(), name_len);
    let index: usize = kani::any();
    kani::assume(index < name_len);
    assert_eq!(parsed.name()[index], name[index]);
    let area = parsed.system_use();
    assert_eq!(area.len(), system_use_len + system_use_len % 2);
    let index: usize = kani::any();
    kani::assume(index < area.len());
    let expected = if index < system_use_len {
        system_use[index]
    } else {
        0
    };
    assert_eq!(area[index], expected);
    kani::cover!(len == 254 && system_use_len % 2 == 1, "full padded record");
}

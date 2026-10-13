use crate::SuspEntries;

const AREA: usize = 32;

#[kani::proof]
#[kani::unwind(10)]
fn susp_walk_stays_in_bounds_and_terminates() {
    let bytes: [u8; AREA] = kani::any();
    let len: usize = kani::any();
    let skip: usize = kani::any();
    kani::assume(len <= AREA);
    let area = &bytes[..len];
    let mut entries = SuspEntries::new(area, skip);
    let mut pos = skip;
    let mut count = 0;
    for _ in 0..=AREA / 4 {
        let Some(entry) = entries.next() else {
            let rest = len.saturating_sub(pos);
            let stopped = rest < 4
                || usize::from(area[pos + 2]) < 4
                || usize::from(area[pos + 2]) > rest
                || (area[pos], area[pos + 1]) == (b'S', b'T');
            assert!(stopped);
            assert!(entries.next().is_none());
            kani::cover!(
                count >= 2 && rest >= 4 && (area[pos], area[pos + 1]) == (b'S', b'T'),
                "two entries then ST"
            );
            kani::cover!(count >= 2 && rest == 0, "entries fill the area");
            return;
        };
        assert!(pos <= len && len - pos >= 4);
        let entry_len = usize::from(area[pos + 2]);
        assert!(entry_len >= 4 && entry_len <= len - pos);
        assert!((area[pos], area[pos + 1]) != (b'S', b'T'));
        assert_eq!(entry.signature, [area[pos], area[pos + 1]]);
        assert_eq!(entry.version, area[pos + 3]);
        assert_eq!(entry.data.len(), entry_len - 4);
        assert!(core::ptr::eq(entry.data.as_ptr(), area[pos + 4..].as_ptr()));
        pos += entry_len;
        count += 1;
    }
    panic!("the walk yielded more entries than four-byte headers fit");
}

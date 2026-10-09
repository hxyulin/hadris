use crate::{ChainGuard, FatKind, RawFsInfo, layout};

#[kani::proof]
fn fat_entry_offsets_fit_valid_cluster_domain() {
    let cluster: u32 = kani::any();
    let wide = u64::from(cluster);
    assert_eq!(FatKind::Fat12.entry_offset(wide), wide + wide / 2);
    assert_eq!(FatKind::Fat16.entry_offset(wide), wide * 2);
    assert_eq!(FatKind::Fat32.entry_offset(wide), wide * 4);
}

#[kani::proof]
#[kani::unwind(17)]
fn chain_guard_detects_all_four_node_graph_cycles() {
    let next: [u8; 4] = kani::any();
    for cluster in next {
        kani::assume(cluster < 4);
    }
    let mut cluster: u8 = kani::any();
    kani::assume(cluster < 4);
    let mut seen = 1u8 << cluster;
    let mut guard = ChainGuard::new(u32::from(cluster));
    let mut detected = false;
    for _ in 0..16 {
        cluster = next[usize::from(cluster)];
        let repeated = seen & (1 << cluster) != 0;
        if !guard.step(u32::from(cluster)) {
            assert!(repeated);
            detected = true;
            break;
        }
        seen |= 1 << cluster;
    }
    assert!(detected);
}

#[kani::proof]
#[kani::unwind(5)]
fn fsinfo_signatures_are_required_and_hints_preserved() {
    let lead: u32 = kani::any();
    let structure: u32 = kani::any();
    let trail: u32 = kani::any();
    let free: u32 = kani::any();
    let next: u32 = kani::any();
    let mut info: RawFsInfo = bytemuck::Zeroable::zeroed();
    info.signature = lead.to_le_bytes();
    info.structure_signature = structure.to_le_bytes();
    info.trail_signature.set(trail);
    assert_eq!(
        crate::check_fs_info(&info).is_ok(),
        lead == 0x4161_5252 && structure == 0x6141_7272 && trail == 0xaa55_0000
    );
    let encoded = layout::encode_fs_info(free, next);
    assert_eq!(
        u32::from_le_bytes(encoded[0..4].try_into().unwrap()),
        0x4161_5252
    );
    assert_eq!(
        u32::from_le_bytes(encoded[484..488].try_into().unwrap()),
        0x6141_7272
    );
    assert_eq!(
        u32::from_le_bytes(encoded[488..492].try_into().unwrap()),
        free
    );
    assert_eq!(
        u32::from_le_bytes(encoded[492..496].try_into().unwrap()),
        next
    );
    assert_eq!(
        u32::from_le_bytes(encoded[508..512].try_into().unwrap()),
        0xaa55_0000
    );
}

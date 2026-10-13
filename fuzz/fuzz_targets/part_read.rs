#![no_main]
//! Fuzz the partition table reader: detect and parse MBR (with EBR chains),
//! GPT (with the backup fallback) and hybrid tables from arbitrary bytes at
//! 512- and 4096-byte blocks, then edit, write and re-read what was found.
//! Arbitrary bytes must never panic/abort/OOM.
//!
//! Oracles (tagged `ORACLE:`): `scan` lists what `read` lists, and a table
//! that `write` accepts reads back with the same partitions.

use core::ops::ControlFlow;

use hadris_part::sync::{open, read, scan, write};
use hadris_part::{Disk, Gpt, GptEntry, Guid, Hybrid, MbrEntry, MbrType, PartitionTable};
use hadris_storage::{BlockSize, MemDevice};
use libfuzzer_sys::fuzz_target;

fn word(data: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = data.get(at + i).copied().unwrap_or(0);
    }
    u64::from_le_bytes(bytes)
}

/// Fuzz-driven edits; each must succeed or fail without a panic, and a
/// failed edit must leave the table unchanged.
fn edit(disk: &mut Disk, data: &[u8]) {
    let start = word(data, 440) % (1 << 20);
    let len = word(data, 448) % (1 << 16);
    let index = (word(data, 456) % 140) as usize;
    match disk.table_mut() {
        PartitionTable::Mbr(mbr) => {
            let before = mbr.clone();
            let kind = MbrType::new(data.get(3).copied().unwrap_or(0x83));
            if mbr.add(MbrEntry::new(kind, start, len)).is_err()
                && mbr.add_logical(MbrEntry::new(kind, start, len)).is_err()
                && mbr.resize(index, len).is_err()
                && mbr.remove(index).is_err()
            {
                assert_eq!(*mbr, before, "ORACLE: failed MBR edits changed the table");
            }
        }
        PartitionTable::Gpt(gpt) => edit_gpt(gpt, data, start, len, index),
        PartitionTable::Hybrid(hybrid) => {
            let mirror = hybrid.mirror();
            edit_gpt(hybrid.gpt_mut(), data, start, len, index);
            if let Ok(rebuilt) = Hybrid::new(hybrid.gpt().clone(), &mirror) {
                assert_eq!(
                    rebuilt.mirror(),
                    mirror,
                    "ORACLE: a rebuilt hybrid MBR has different mirrors"
                );
                *hybrid = rebuilt;
            }
        }
        _ => {}
    }
}

fn edit_gpt(gpt: &mut Gpt, data: &[u8], start: u64, len: u64, index: usize) {
    let before = gpt.clone();
    let entry = GptEntry::new(Guid::from_bytes([0xA5; 16]), Guid::NIL, start, len);
    if gpt.add(entry).is_err() && gpt.resize(index, len).is_err() && gpt.remove(index).is_err() {
        assert_eq!(*gpt, before, "ORACLE: failed GPT edits changed the table");
    }
    let before = gpt.clone();
    let count = (word(data, 464) % 512) as usize;
    if gpt.set_entry_count(count).is_err() {
        assert_eq!(
            *gpt, before,
            "ORACLE: a failed entry count change changed the table"
        );
    }
    let before = gpt.clone();
    let blocks = gpt
        .block_count()
        .saturating_add_signed(word(data, 472) as i8 as i64);
    if gpt.relocate_backup(blocks).is_err() {
        assert_eq!(
            *gpt, before,
            "ORACLE: a failed backup move changed the table"
        );
    } else {
        assert_eq!(
            gpt.backup_lba(),
            blocks - 1,
            "ORACLE: the backup header did not move"
        );
    }
}

fn drive(data: &[u8]) {
    for size in [512u32, 4096] {
        let block_size = BlockSize::new(size).unwrap();
        let mut dev = MemDevice::new(data, block_size);
        let Ok(disk) = read(&mut dev) else {
            continue;
        };
        let listed: Vec<_> = disk.partitions().collect();
        let mut scanned = Vec::new();
        let kind = scan(&mut dev, |p| {
            scanned.push(p);
            ControlFlow::Continue(())
        })
        .expect("ORACLE: scan failed where read succeeded");
        assert_eq!(
            kind,
            disk.table().kind(),
            "ORACLE: scan and read disagree on the table"
        );
        assert_eq!(
            scanned, listed,
            "ORACLE: scan and read disagree on the partitions"
        );

        for partition in &listed {
            let _ = (
                partition.slot(),
                partition.end(),
                partition.size_bytes(),
                partition.kind(),
                partition.flags(),
                partition.unique_guid(),
                partition.name().map(|name| name.to_string()),
            );
            let _ = open(&mut dev, partition);
        }
        if let PartitionTable::Hybrid(hybrid) = disk.table() {
            let _ = hybrid.mbr_partitions().count();
        }

        let mut copy = MemDevice::new(data.to_vec(), block_size);
        if write(&mut copy, &disk).is_ok() {
            let back = read(&mut copy).expect("ORACLE: a written table does not read back");
            assert_eq!(
                back.partitions().collect::<Vec<_>>(),
                listed,
                "ORACLE: a written table reads back different partitions"
            );
        }

        let mut edited = disk.clone();
        edit(&mut edited, data);
        let mut copy = MemDevice::new(data.to_vec(), block_size);
        if write(&mut copy, &edited).is_ok() {
            let back = read(&mut copy).expect("ORACLE: an edited table does not read back");
            assert_eq!(
                back.partitions().collect::<Vec<_>>(),
                edited.partitions().collect::<Vec<_>>(),
                "ORACLE: an edited table reads back different partitions"
            );
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});

//! Regression tests for the 2026-08 hadris-fat correctness audit, exFAT write
//! path (`unstable-exfat` preview). Each test asserts the CORRECT/SAFE
//! behavior and is named after the audit finding it guards.

#![cfg(all(feature = "unstable-exfat", feature = "write", feature = "std"))]

use std::fs::OpenOptions;
use std::io::Seek as _;
use std::path::Path;
use tempfile::TempDir;

use hadris_fat::exfat::{ExFatFormatOptions, ExFatVolume, format_exfat};
use hadris_fat::io::{Read as HadrisRead, Write as HadrisWrite};

/// Build a fresh, formatted exFAT image at `path` of `size` bytes.
fn make_image(path: &Path, size: u64, label: &str) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .expect("create image file");
    file.set_len(size).expect("set image length");

    let opts = ExFatFormatOptions::default().volume_label(label);
    format_exfat(&mut file, size, &opts).expect("format_exfat");
    file.sync_all().expect("sync");
}

fn open_image(path: &Path) -> std::fs::File {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open image");
    file.seek(std::io::SeekFrom::Start(0)).unwrap();
    file
}

fn read_root_file(image_path: &Path, name: &str) -> Vec<u8> {
    let file = open_image(image_path);
    let fs = ExFatVolume::open(file).expect("reopen exFAT");
    let mut reader = fs.open_file(name).expect("open_file");

    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = HadrisRead::read(&mut reader, &mut buf).expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

// X1: extending a file across a cluster boundary onto a fragmented layout must
// write the FAT links so the whole chain is reachable on read-back. Previously
// `allocate_next_cluster` allocated a cluster but never wrote `FAT[prev]=new`,
// nor linked the contiguous prefix, so everything past the first cluster was
// lost.
#[test]
fn x1_cross_cluster_extend_preserves_data() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("x1.img");
    make_image(&img, 32 * 1024 * 1024, "X1");

    let cluster_size = {
        let file = open_image(&img);
        let fs = ExFatVolume::open(file).expect("open");
        fs.info().bytes_per_cluster
    };

    // Two distinct, position-dependent patterns so any truncation or
    // corruption is visible on read-back.
    let a_payload: Vec<u8> = (0..(2 * cluster_size))
        .map(|i| (i.wrapping_mul(7).wrapping_add(3)) as u8)
        .collect();
    let b_payload: Vec<u8> = vec![0xB5u8; cluster_size];

    {
        let file = open_image(&img);
        let fs = ExFatVolume::open(file).expect("open");

        let a_entry = {
            let root = fs.root_dir();
            fs.create_file(&root, "a.bin").expect("create a")
        };
        let b_entry = {
            let root = fs.root_dir();
            fs.create_file(&root, "b.bin").expect("create b")
        };

        let mut wa = fs.write_file(&a_entry).expect("writer a");
        let mut wb = fs.write_file(&b_entry).expect("writer b");

        // 1. Fill a's first cluster only (allocates cluster C, no next).
        wa.write_all(&a_payload[..cluster_size]).expect("a first");
        // 2. Give b the immediately-adjacent cluster C+1.
        wb.write_all(&b_payload).expect("b");
        // 3. Extend a across the boundary: C+1 is taken, forcing the
        //    fragmented path in allocate_next_cluster.
        wa.write_all(&a_payload[cluster_size..]).expect("a second");

        wa.finish().expect("finish a");
        wb.finish().expect("finish b");
    }

    let got = read_root_file(&img, "a.bin");
    assert_eq!(
        got.len(),
        a_payload.len(),
        "read-back length mismatch: data past the first cluster was lost \
         (got {} bytes, expected {})",
        got.len(),
        a_payload.len()
    );
    assert_eq!(
        got, a_payload,
        "read-back content mismatch after cross-cluster extend"
    );
}

// X2: allocation is authoritative against the bitmap. The contiguous fast-path
// marks only the bitmap (no FAT write); a fragmented fallback that scanned the
// FAT for zeros would re-hand-out those bitmap-allocated clusters. On a full
// volume, allocation must report no free space rather than double-allocate.
#[test]
fn x2_no_double_allocation_when_full() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("x2.img");
    make_image(&img, 32 * 1024 * 1024, "X2");

    let file = open_image(&img);
    let fs = ExFatVolume::open(file).expect("open");

    let free0 = fs.free_cluster_count();
    assert!(free0 > 0, "fresh volume should have free clusters");

    // Consume every free cluster via the contiguous fast-path (bitmap-only).
    let (region_first, contiguous) = fs
        .allocate_clusters(free0, 2)
        .expect("bulk contiguous allocation");
    assert!(
        contiguous,
        "expected the contiguous fast-path (bitmap-only, no FAT writes)"
    );
    assert_eq!(
        fs.free_cluster_count(),
        0,
        "volume should now be full per the bitmap"
    );

    // The volume is genuinely full. A correct allocator must refuse.
    let result = fs.allocate_clusters(1, 2);
    if let Ok((cluster, _)) = result {
        let already = fs.is_cluster_allocated(cluster).unwrap();
        panic!(
            "allocate_clusters handed out cluster {cluster} on a full volume \
             (region started at {region_first}, is_cluster_allocated={already}) \
             — double allocation via FAT-scan fallback"
        );
    }
    assert!(
        result.is_err(),
        "expected NoFreeSpace on a full volume, got {result:?}"
    );
}

// X3: mutations that change the allocation bitmap (here `create_dir`) must
// persist it. Previously only the file writer's `finish` flushed the bitmap, so
// a directory's cluster allocation was lost on remount.
#[test]
fn x3_directory_allocation_persists_across_remount() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("x3.img");
    make_image(&img, 32 * 1024 * 1024, "X3");

    let free_before = {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        fs.free_cluster_count()
    };

    {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        let root = fs.root_dir();
        fs.create_dir(&root, "sub").expect("create_dir");
        // No explicit flush: create_dir must persist the bitmap itself.
    }

    let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
    assert_eq!(
        fs.free_cluster_count(),
        free_before - 1,
        "create_dir's cluster allocation was not persisted to the bitmap"
    );
}

// X5: truncating a fragmented file must free the dropped tail clusters in the
// allocation bitmap, not only in the FAT. Previously the fragmented branch
// called `fat.truncate_chain` (FAT-only), leaking the clusters in the bitmap.
#[test]
fn x5_fragmented_truncate_frees_bitmap() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("x5.img");
    make_image(&img, 32 * 1024 * 1024, "X5");

    let cs = {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        fs.info().bytes_per_cluster
    };

    // Build a fragmented, multi-cluster file "a" by interleaving writes with a
    // second file "b" so a's clusters cannot stay adjacent.
    {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        let a = {
            let r = fs.root_dir();
            fs.create_file(&r, "a.bin").unwrap()
        };
        let b = {
            let r = fs.root_dir();
            fs.create_file(&r, "b.bin").unwrap()
        };
        let mut wa = fs.write_file(&a).unwrap();
        let mut wb = fs.write_file(&b).unwrap();
        let pat_a = vec![0xA7u8; 3 * cs];
        let pat_b = vec![0xB3u8; 2 * cs];
        wa.write_all(&pat_a[..cs]).unwrap();
        wb.write_all(&pat_b[..cs]).unwrap();
        wa.write_all(&pat_a[cs..2 * cs]).unwrap();
        wb.write_all(&pat_b[cs..]).unwrap();
        wa.write_all(&pat_a[2 * cs..]).unwrap();
        wa.finish().unwrap();
        wb.finish().unwrap();
    }

    let free_with_full_a = {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        let a = fs.root_dir().find("a.bin").unwrap().expect("a exists");
        assert!(
            !a.no_fat_chain,
            "precondition: 'a' must be fragmented (chained), not contiguous"
        );
        fs.free_cluster_count()
    };

    // Truncate "a" down to one cluster: two fragmented tail clusters must be
    // freed in the bitmap.
    {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        let a = fs.root_dir().find("a.bin").unwrap().expect("a exists");
        fs.truncate(&a, cs as u64).expect("truncate");
    }

    let free_after_truncate = {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        fs.free_cluster_count()
    };

    assert!(
        free_after_truncate > free_with_full_a,
        "fragmented truncate did not reclaim tail clusters in the bitmap \
         (free stayed at {free_with_full_a})"
    );
    assert_eq!(
        free_after_truncate,
        free_with_full_a + 2,
        "expected exactly 2 tail clusters reclaimed"
    );
}

// X6: overwriting a contiguous, multi-cluster file in place must reuse the
// file's own already-allocated clusters. Previously, crossing a cluster
// boundary during an overwrite saw the file's next cluster as "already
// allocated" and allocated a brand-new cluster instead, orphaning the old one
// (a bitmap leak) and needlessly fragmenting the file (the issue #90 pattern).
#[test]
fn x6_contiguous_overwrite_reuses_clusters() {
    let tmp = TempDir::new().unwrap();
    let img = tmp.path().join("x6.img");
    make_image(&img, 32 * 1024 * 1024, "X6");

    let cs = {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        fs.info().bytes_per_cluster
    };

    // Create a contiguous 3-cluster file with no interleaving so it stays
    // contiguous (no_fat_chain).
    let original: Vec<u8> = (0..(3 * cs)).map(|i| (i % 251) as u8).collect();
    {
        let fs = ExFatVolume::open(open_image(&img)).expect("open");
        let a = {
            let r = fs.root_dir();
            fs.create_file(&r, "a.bin").unwrap()
        };
        let mut w = fs.write_file(&a).unwrap();
        w.write_all(&original).unwrap();
        w.finish().unwrap();
    }

    let free_before = {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        let a = fs.root_dir().find("a.bin").unwrap().expect("a exists");
        assert!(
            a.no_fat_chain,
            "precondition: 'a' must be contiguous (no_fat_chain) before overwrite"
        );
        fs.free_cluster_count()
    };

    // Overwrite the whole file in place with a distinct, same-size pattern.
    let replacement: Vec<u8> = (0..(3 * cs)).map(|i| (i % 251) as u8 ^ 0xFF).collect();
    {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        let a = fs.root_dir().find("a.bin").unwrap().expect("a exists");
        let mut w = fs.write_file(&a).unwrap();
        w.write_all(&replacement).unwrap();
        w.finish().unwrap();
    }

    let free_after = {
        let fs = ExFatVolume::open(open_image(&img)).expect("reopen");
        let a = fs.root_dir().find("a.bin").unwrap().expect("a exists");
        assert!(
            a.no_fat_chain,
            "overwrite fragmented a contiguous file instead of reusing its clusters"
        );
        fs.free_cluster_count()
    };

    assert_eq!(
        free_after,
        free_before,
        "in-place overwrite allocated new clusters (leaked {} cluster(s))",
        free_before as i64 - free_after as i64
    );

    let got = read_root_file(&img, "a.bin");
    assert_eq!(got.len(), replacement.len(), "read-back length mismatch");
    assert_eq!(
        got, replacement,
        "read-back content mismatch after overwrite"
    );
}

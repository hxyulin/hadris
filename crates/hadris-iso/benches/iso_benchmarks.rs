//! Criterion benchmarks for hadris-iso
//!
//! Run with: cargo bench -p hadris-iso

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use hadris_iso::boot::{BootCatalog, BootSectionEntry, EmulationType, PlatformId};
use hadris_iso::joliet::{decode_joliet_name, encode_joliet_name};
use hadris_iso::rrip::RripBuilder;
use hadris_iso::susp::{SystemUseBuilder, SystemUseIter};

/// Check if xorriso is available
fn xorriso_available() -> bool {
    Command::new("xorriso")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create test files in a directory
fn create_test_files(dir: &Path, count: usize, size: usize) {
    for i in 0..count {
        let filename = format!("file_{:04}.dat", i);
        let content: Vec<u8> = (0..size).map(|j| ((i + j) % 256) as u8).collect();
        fs::write(dir.join(&filename), &content).unwrap();
    }
}

/// Create an ISO using xorriso
fn create_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    Command::new("xorriso")
        .args([
            "-as", "mkisofs",
            "-o", iso_path.to_str().unwrap(),
            "-V", "BENCHMARK",
            "-J", "-R",
            content_dir.to_str().unwrap(),
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Benchmark Joliet encoding
fn bench_joliet_encode(c: &mut Criterion) {
    let test_strings = [
        "simple.txt",
        "test_file_with_longer_name.txt",
        "日本語ファイル.txt",
        "文件名称很长的中文文档.doc",
        "한국어_파일명.xlsx",
    ];

    let mut group = c.benchmark_group("joliet_encode");

    for name in &test_strings {
        group.bench_with_input(BenchmarkId::from_parameter(name), name, |b, input| {
            b.iter(|| encode_joliet_name(black_box(input)));
        });
    }

    group.finish();
}

/// Benchmark Joliet decoding
fn bench_joliet_decode(c: &mut Criterion) {
    let test_strings = [
        "simple.txt",
        "test_file_with_longer_name.txt",
        "日本語ファイル.txt",
        "文件名称很长的中文文档.doc",
    ];

    let encoded: Vec<(&str, Vec<u8>)> = test_strings
        .iter()
        .map(|s| (*s, encode_joliet_name(s)))
        .collect();

    let mut group = c.benchmark_group("joliet_decode");

    for (name, data) in &encoded {
        group.bench_with_input(BenchmarkId::from_parameter(name), data, |b, input| {
            b.iter(|| decode_joliet_name(black_box(input)));
        });
    }

    group.finish();
}

/// Benchmark RRIP builder
fn bench_rrip_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("rrip_builder");

    group.bench_function("minimal", |b| {
        b.iter(|| {
            let mut builder = RripBuilder::new();
            builder.add_px(0o100644, 1, 1000, 1000, 1);
            builder.build()
        });
    });

    group.bench_function("complete", |b| {
        b.iter(|| {
            let mut builder = RripBuilder::new();
            builder
                .add_sp(0)
                .add_rrip_er()
                .add_px(0o100644, 1, 1000, 1000, 1)
                .add_nm(b"test_file.txt")
                .add_st();
            builder.build()
        });
    });

    group.bench_function("with_symlink", |b| {
        b.iter(|| {
            let mut builder = RripBuilder::new();
            builder
                .add_px(0o120777, 1, 1000, 1000, 1)
                .add_nm(b"symlink")
                .add_sl("/usr/lib/libtest.so");
            builder.build()
        });
    });

    group.finish();
}

/// Benchmark SUSP iterator
fn bench_susp_iterator(c: &mut Criterion) {
    // Build test data
    let mut builder = SystemUseBuilder::new();
    builder
        .add_sp(0)
        .add_er("RRIP_1991A", "Rock Ridge", "IEEE P1282", 1)
        .add_padding(4)
        .add_st();
    let small_data = builder.build();

    let mut builder = SystemUseBuilder::new();
    builder.add_sp(0).add_er("RRIP_1991A", "Rock Ridge", "IEEE P1282", 1);
    for _ in 0..10 {
        builder.add_padding(8);
    }
    builder.add_st();
    // Need to create a new builder for RripBuilder
    let mut rrip_builder = RripBuilder::new();
    rrip_builder.add_sp(0).add_rrip_er();
    let large_data = rrip_builder.build();

    let mut group = c.benchmark_group("susp_iterator");

    group.throughput(Throughput::Bytes(small_data.len() as u64));
    group.bench_with_input(BenchmarkId::new("small", small_data.len()), &small_data, |b, data| {
        b.iter(|| {
            let iter = SystemUseIter::new(black_box(data), 0);
            let count: usize = iter.count();
            count
        });
    });

    group.throughput(Throughput::Bytes(large_data.len() as u64));
    group.bench_with_input(BenchmarkId::new("large", large_data.len()), &large_data, |b, data| {
        b.iter(|| {
            let iter = SystemUseIter::new(black_box(data), 0);
            let count: usize = iter.count();
            count
        });
    });

    group.finish();
}

/// Benchmark boot catalog operations
fn bench_boot_catalog(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot_catalog");

    group.bench_function("create_default", |b| {
        b.iter(|| {
            BootCatalog::default()
        });
    });

    group.bench_function("create_with_section", |b| {
        b.iter(|| {
            let mut catalog = BootCatalog::default();
            catalog.add_section(
                PlatformId::X80X86,
                vec![BootSectionEntry::new(EmulationType::NoEmulation, 0x07C0, 4, 20)],
            );
            catalog
        });
    });

    group.bench_function("serialize", |b| {
        let mut catalog = BootCatalog::default();
        catalog.add_section(
            PlatformId::X80X86,
            vec![BootSectionEntry::new(EmulationType::NoEmulation, 0x07C0, 4, 20)],
        );

        b.iter(|| {
            let mut buf = Vec::with_capacity(256);
            catalog.write(&mut buf).unwrap();
            buf
        });
    });

    group.bench_function("roundtrip", |b| {
        let mut catalog = BootCatalog::default();
        catalog.add_section(
            PlatformId::X80X86,
            vec![BootSectionEntry::new(EmulationType::NoEmulation, 0x07C0, 4, 20)],
        );
        let mut buf = Vec::new();
        catalog.write(&mut buf).unwrap();

        b.iter(|| {
            let mut cursor = Cursor::new(black_box(&buf));
            BootCatalog::parse(&mut cursor).unwrap()
        });
    });

    group.finish();
}

/// Benchmark ISO image opening (requires xorriso)
fn bench_iso_open(c: &mut Criterion) {
    if !xorriso_available() {
        eprintln!("Skipping ISO open benchmarks: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();

    // Create small ISO (10 files, 1KB each)
    let small_content = temp_dir.path().join("small_content");
    let small_iso = temp_dir.path().join("small.iso");
    fs::create_dir(&small_content).unwrap();
    create_test_files(&small_content, 10, 1024);
    assert!(create_iso_with_xorriso(&small_content, &small_iso));
    let small_data = fs::read(&small_iso).unwrap();

    // Create medium ISO (50 files, 8KB each)
    let medium_content = temp_dir.path().join("medium_content");
    let medium_iso = temp_dir.path().join("medium.iso");
    fs::create_dir(&medium_content).unwrap();
    create_test_files(&medium_content, 50, 8192);
    assert!(create_iso_with_xorriso(&medium_content, &medium_iso));
    let medium_data = fs::read(&medium_iso).unwrap();

    // Create large ISO (100 files, 64KB each)
    let large_content = temp_dir.path().join("large_content");
    let large_iso = temp_dir.path().join("large.iso");
    fs::create_dir(&large_content).unwrap();
    create_test_files(&large_content, 100, 65536);
    assert!(create_iso_with_xorriso(&large_content, &large_iso));
    let large_data = fs::read(&large_iso).unwrap();

    let mut group = c.benchmark_group("iso_open");

    group.throughput(Throughput::Bytes(small_data.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("small", format!("{}KB", small_data.len() / 1024)),
        &small_data,
        |b, data| {
            b.iter(|| {
                let cursor = Cursor::new(black_box(data.clone()));
                hadris_iso::read::IsoImage::open(cursor).unwrap()
            });
        },
    );

    group.throughput(Throughput::Bytes(medium_data.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("medium", format!("{}KB", medium_data.len() / 1024)),
        &medium_data,
        |b, data| {
            b.iter(|| {
                let cursor = Cursor::new(black_box(data.clone()));
                hadris_iso::read::IsoImage::open(cursor).unwrap()
            });
        },
    );

    group.throughput(Throughput::Bytes(large_data.len() as u64));
    group.bench_with_input(
        BenchmarkId::new("large", format!("{}MB", large_data.len() / 1024 / 1024)),
        &large_data,
        |b, data| {
            b.iter(|| {
                let cursor = Cursor::new(black_box(data.clone()));
                hadris_iso::read::IsoImage::open(cursor).unwrap()
            });
        },
    );

    group.finish();
}

/// Benchmark directory traversal (requires xorriso)
fn bench_directory_traversal(c: &mut Criterion) {
    if !xorriso_available() {
        eprintln!("Skipping directory traversal benchmarks: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("test.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_files(&content_dir, 20, 1024);

    assert!(create_iso_with_xorriso(&content_dir, &iso_path));

    let iso_data = fs::read(&iso_path).unwrap();
    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).unwrap();

    let mut group = c.benchmark_group("directory_traversal");

    group.bench_function("root_entries", |b| {
        b.iter(|| {
            let root = image.root_dir();
            let dir = root.iter(&image);
            let mut count = 0;
            for entry in dir.entries() {
                let _ = black_box(entry.unwrap());
                count += 1;
            }
            count
        });
    });

    group.bench_function("read_pvd", |b| {
        b.iter(|| {
            image.read_pvd()
        });
    });

    group.bench_function("volume_descriptors", |b| {
        b.iter(|| {
            let mut count = 0;
            for vd in image.read_volume_descriptors() {
                let _ = black_box(vd.unwrap());
                count += 1;
            }
            count
        });
    });

    group.finish();
}

/// Benchmark validation entry checksum
fn bench_checksum(c: &mut Criterion) {
    use hadris_iso::boot::BootValidationEntry;

    let entry = BootValidationEntry::new();

    c.bench_function("validation_checksum", |b| {
        b.iter(|| {
            entry.calculate_checksum()
        });
    });
}

criterion_group!(
    benches,
    bench_joliet_encode,
    bench_joliet_decode,
    bench_rrip_builder,
    bench_susp_iterator,
    bench_boot_catalog,
    bench_checksum,
    bench_iso_open,
    bench_directory_traversal,
);

criterion_main!(benches);

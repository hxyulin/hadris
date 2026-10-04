#[path = "../tests/common/exfat.rs"]
mod common;
#[allow(dead_code)]
mod support;

use hadris_fat::embedded::MountToken;
use hadris_fat::exfat::embedded::sync::ExFat;
use hadris_fat::exfat::sync::ExFatFs;
use hadris_fs::sync::FileSystem;
use hadris_fs::{MountOptions, Name, OpenOptions, SetAttr};
use std::cell::Cell;
use std::hint::black_box;
use std::time::Instant;
use support::{Counted, IoCounts};

const BYTES: usize = 1 << 20;

fn sample(
    image: &[u8],
    data: &[u8],
    driver: &str,
    workload: &str,
    verify: bool,
) -> (u128, IoCounts, Vec<u8>) {
    let counts = Cell::new(IoCounts::default());
    let dev = Counted {
        inner: common::device(image.to_vec(), 512),
        counts: &counts,
        written_blocks: None,
    };
    let mut buf = vec![0; 65536];
    let (elapsed, dev) = if driver == "embedded" {
        let mut token = MountToken::new();
        let mut fs: ExFat<_> = ExFat::mount(dev, &mut token).unwrap();
        let file = fs
            .open(fs.root(), "DATA.BIN", OpenOptions::new().read())
            .unwrap();
        counts.set(IoCounts::default());
        let start = Instant::now();
        let mut offset = 0;
        loop {
            let n = fs.read(&file, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            if verify {
                assert_eq!(&buf[..n], &data[offset..offset + n]);
            }
            black_box(&buf[..n]);
            offset += n;
        }
        assert_eq!(offset, BYTES);
        (start.elapsed().as_nanos(), fs.unmount())
    } else {
        let mut fs = ExFatFs::mount(dev, MountOptions::new()).unwrap();
        let node = if workload.starts_with("write") {
            fs.create(fs.root(), Name::new("DATA.BIN"), &SetAttr::new())
                .unwrap()
        } else {
            fs.lookup(fs.root(), Name::new("DATA.BIN")).unwrap()
        };
        counts.set(IoCounts::default());
        let start = Instant::now();
        let chunk = if workload == "write-512" { 512 } else { 65536 };
        let mut offset = 0;
        while offset < BYTES {
            let n = if workload.starts_with("write") {
                fs.write(
                    node,
                    offset as u64,
                    &data[offset..(offset + chunk).min(BYTES)],
                )
                .unwrap()
            } else {
                let n = fs.read(node, offset as u64, &mut buf).unwrap();
                if verify {
                    assert_eq!(&buf[..n], &data[offset..offset + n]);
                }
                black_box(&buf[..n]);
                n
            };
            assert!(n > 0);
            offset += n;
        }
        if workload.starts_with("write") {
            fs.sync().unwrap();
        }
        (start.elapsed().as_nanos(), fs.into_inner())
    };
    (elapsed, counts.get(), dev.inner.into_inner())
}

fn main() {
    let samples = std::env::args()
        .nth(1)
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(7);
    assert!(samples > 0);
    let data = (0..BYTES)
        .map(|i| ((i as u32).wrapping_mul(0x9e3779b9) >> 17) as u8)
        .collect::<Vec<_>>();
    println!(
        "cluster_bytes,allocation,driver,workload,median_ns,reads,writes,read_bytes,write_bytes,flushes"
    );
    for cluster in [512, 4096] {
        let blank = common::image(common::small(8 << 20, cluster));
        let mut fs = common::mount(&blank);
        let root = fs.root();
        common::write(&mut fs, root, "DATA.BIN", &data);
        fs.sync().unwrap();
        let chained = common::image(fs);
        let mut contiguous = chained.clone();
        let geo = common::Geometry::of(&contiguous);
        let set = geo.set(&contiguous, geo.root, "DATA.BIN");
        geo.unchain(&mut contiguous, &set);
        let mut fragmented = chained.clone();
        let first = common::le32(&fragmented, set[1] + 20);
        let clusters = geo.chain(&fragmented, first).len();
        for i in (1..clusters).step_by(2) {
            geo.relocate(&mut fragmented, first, i, geo.count + 1 - i as u32);
        }
        for (allocation, image) in [
            ("chained", &chained),
            ("contiguous", &contiguous),
            ("fragmented", &fragmented),
        ] {
            for driver in ["hosted", "embedded"] {
                run(
                    image, &data, cluster, allocation, driver, "read-64k", samples,
                );
            }
        }
        for workload in ["write-512", "write-64k"] {
            run(
                &blank, &data, cluster, "chained", "hosted", workload, samples,
            );
        }
    }
}

fn run(
    image: &[u8],
    data: &[u8],
    cluster: u32,
    allocation: &str,
    driver: &str,
    workload: &str,
    samples: usize,
) {
    let (_, expected, warm) = sample(image, data, driver, workload, true);
    let mut dev = common::device(warm.clone(), 512);
    assert!(common::check_dev(&mut dev, 4096).1.is_empty());
    assert_eq!(common::read(&mut common::mount(&warm), "DATA.BIN"), data);
    let mut times = Vec::new();
    for _ in 0..samples {
        let (elapsed, counts, _) = sample(image, data, driver, workload, false);
        assert_eq!(counts, expected);
        times.push(elapsed);
    }
    times.sort_unstable();
    println!(
        "{cluster},{allocation},{driver},{workload},{},{},{},{},{},{}",
        times[samples / 2],
        expected.read_calls,
        expected.write_calls,
        expected.read_bytes,
        expected.write_bytes,
        expected.flush_calls
    );
}

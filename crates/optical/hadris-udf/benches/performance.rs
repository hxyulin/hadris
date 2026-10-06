#[path = "support/fixtures.rs"]
mod fixtures;

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, MountOptions, Name};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, sync::BlockDevice};
use hadris_udf::sync::UdfFs;
use std::{
    cell::Cell,
    hint::black_box,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    calls: u64,
    bytes: u64,
    aed_calls: u64,
    aed_bytes: u64,
}
struct Counted {
    inner: MemDevice<Vec<u8>>,
    counts: Rc<Cell<Counts>>,
    aeds: Rc<Vec<u64>>,
}
impl hadris_io::ErrorType for Counted {
    type Error = core::convert::Infallible;
}
impl BlockDevice for Counted {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.read_blocks(first, buf)?;
        let mut counts = self.counts.get();
        counts.calls += 1;
        counts.bytes += buf.len() as u64;
        let start = first.get();
        let end = start + (buf.len() / self.block_size().get() as usize) as u64;
        let covered = self.aeds.partition_point(|&block| block < end)
            - self.aeds.partition_point(|&block| block < start);
        if covered > 0 {
            counts.aed_calls += 1;
            counts.aed_bytes += covered as u64 * u64::from(self.block_size().get());
        }
        self.counts.set(counts);
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Op {
    Mount,
    List,
    Last,
    Miss,
    Stat,
    Read(usize, bool),
    Scattered,
}
struct Settings {
    samples: usize,
    cache: usize,
    profile: u64,
}

fn operate<D: BlockDevice>(
    fs: &mut UdfFs<D>,
    fixture: &fixtures::Fixture,
    op: Op,
    node: Option<hadris_fs::NodeId>,
    buf: &mut [u8],
) {
    match op {
        Op::Mount => unreachable!(),
        Op::List => {
            let mut cursor = DirCursor::START;
            let mut n = 0;
            while let Some(entry) = fs.readdir(fs.root(), cursor).unwrap() {
                cursor = entry.next_cursor();
                assert_eq!(entry.name().as_bytes(), fixture.names[n]);
                n += 1;
                black_box(entry);
            }
            assert_eq!(n, fixture.names.len());
        }
        Op::Last => {
            black_box(
                fs.lookup(
                    fs.root(),
                    Name::new(&format!("entry-{:04}", fixture.entries - 1)),
                )
                .unwrap(),
            );
        }
        Op::Miss => {
            assert_eq!(
                fs.lookup(fs.root(), Name::new("absent"))
                    .unwrap_err()
                    .kind(),
                hadris_fs::ErrorKind::NotFound
            );
        }
        Op::Stat => {
            for _ in 0..100 {
                black_box(fs.stat(node.unwrap()).unwrap());
            }
        }
        Op::Read(_, unaligned) => {
            let mut pos = u64::from(unaligned);
            while pos < 1024 * 1024 {
                let n = fs.read(node.unwrap(), pos, buf).unwrap();
                assert!(n > 0);
                assert_eq!(&buf[..n], &fixture.payload[pos as usize..pos as usize + n]);
                black_box(&buf[..n]);
                pos += n as u64;
            }
        }
        Op::Scattered => {
            for i in 0..256 {
                let offset = ((i * 103) % 256) * 4096;
                assert_eq!(fs.read(node.unwrap(), offset, buf).unwrap(), 4096);
                assert_eq!(
                    buf,
                    &fixture.payload[offset as usize..offset as usize + buf.len()]
                );
                black_box(&buf);
            }
        }
    }
}

fn row<D: BlockDevice>(
    fixture: &fixtures::Fixture,
    block: u32,
    label: &str,
    op: Op,
    settings: &Settings,
    make: impl Fn(Counted) -> D,
) {
    let Settings {
        samples,
        cache,
        profile,
    } = *settings;
    assert!(
        profile == 0 || !matches!(op, Op::Mount),
        "profile a traversal or read workload"
    );
    let entries = fixture.entries;
    let factor = 2048 / u64::from(block);
    let aeds: Rc<Vec<u64>> = Rc::new(
        fixture
            .aeds
            .iter()
            .flat_map(|&at| (at * factor)..((at + 1) * factor))
            .collect(),
    );
    let mut times = Vec::new();
    let mut counts = None;
    for sample in 0..=samples {
        let io = Rc::new(Cell::new(Counts::default()));
        let device = || {
            make(Counted {
                inner: MemDevice::new(fixture.bytes.clone(), BlockSize::new(block).unwrap()),
                counts: io.clone(),
                aeds: aeds.clone(),
            })
        };
        let mut fs = UdfFs::mount(device(), MountOptions::new()).unwrap();
        let node = match op {
            Op::Stat | Op::Read(_, _) | Op::Scattered => {
                Some(fs.lookup(fs.root(), Name::new("payload")).unwrap())
            }
            _ => None,
        };
        let mut buf = vec![
            0;
            match op {
                Op::Read(n, _) => n,
                _ => 4096,
            }
        ];
        let mount_dev = matches!(op, Op::Mount).then(device);
        io.set(Counts::default());
        let mut previous = None;
        let start = Instant::now();
        if matches!(op, Op::Mount) {
            previous = Some(core::mem::replace(
                &mut fs,
                UdfFs::mount(mount_dev.unwrap(), MountOptions::new()).unwrap(),
            ));
            black_box(fs.info());
        } else {
            operate(&mut fs, fixture, op, node, &mut buf);
        }
        let elapsed = start.elapsed().as_nanos();
        drop(previous);
        if sample > 0 {
            times.push(elapsed);
            let current = io.get();
            if let Some(previous) = counts {
                assert!(previous == current);
            }
            counts = Some(current);
        }
        if sample == samples && profile > 0 {
            eprintln!(
                "PROFILE_READY {} {entries}/{block}/{label}",
                std::process::id()
            );
            let until = Instant::now() + Duration::from_secs(profile);
            while Instant::now() < until {
                operate(&mut fs, fixture, op, node, &mut buf);
            }
        }
    }
    times.sort_unstable();
    let counts = counts.unwrap();
    println!(
        "{entries}/{block}/{label},{cache},{},{},{},{},{}",
        times[times.len() / 2],
        counts.calls,
        counts.bytes,
        counts.aed_calls,
        counts.aed_bytes
    );
}
fn main() {
    let samples: usize =
        std::env::var("HADRIS_UDF_BENCH_SAMPLES").map_or(7, |s| s.parse().unwrap());
    assert!(samples > 0);
    let filter = std::env::var("HADRIS_UDF_BENCH_FILTER").unwrap_or_default();
    let cache: usize =
        std::env::var("HADRIS_UDF_BENCH_CACHE_BLOCKS").map_or(0, |s| s.parse().unwrap());
    let profile =
        std::env::var("HADRIS_UDF_BENCH_PROFILE_SECONDS").map_or(0, |s| s.parse().unwrap());
    assert!(
        profile == 0 || !filter.is_empty(),
        "profiling requires an exact case filter"
    );
    let settings = Settings {
        samples,
        cache,
        profile,
    };
    println!("case,cache_blocks,median_ns,read_calls,read_bytes,aed_read_calls,aed_read_bytes");
    let mut selected = 0;
    for entries in [32, 1000] {
        for layout in ["contiguous", "fragmented-inline", "fragmented-aed"] {
            let fixture = fixtures::image(entries, layout);
            if let Some(dir) = std::env::var_os("HADRIS_UDF_BENCH_EXPORT_DIR") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{entries}-{layout}.udf")), &fixture.bytes)
                    .unwrap();
            }
            for block in [512, 2048] {
                for (label, op) in [
                    ("mount", Op::Mount),
                    ("list", Op::List),
                    ("lookup-last", Op::Last),
                    ("lookup-miss", Op::Miss),
                    ("stat-100", Op::Stat),
                    ("read-4k", Op::Read(4096, false)),
                    ("read-4k-unaligned", Op::Read(4096, true)),
                    ("read-64k", Op::Read(65536, false)),
                    ("read-scattered", Op::Scattered),
                ] {
                    let label = if fixture.layout == "contiguous" {
                        label.to_string()
                    } else {
                        format!("{}/{label}", fixture.layout)
                    };
                    let case = format!("{entries}/{block}/{label}");
                    if (profile == 0 && case.contains(&filter)) || (profile > 0 && case == filter) {
                        if cache == 0 {
                            row(&fixture, block, &label, op, &settings, |dev| dev);
                        } else {
                            row(&fixture, block, &label, op, &settings, |dev| {
                                hadris_storage::sync::Cache::new(dev, cache)
                            });
                        }
                        selected += 1;
                    }
                }
            }
        }
    }
    assert!(selected > 0, "filter matched no cases");
}

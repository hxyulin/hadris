use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, DirCursor, MountOptions, Name, Node, Tree};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, sync::BlockDevice};
use hadris_udf::{UdfOptions, sync::UdfFs};
use std::{hint::black_box, time::Instant};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    calls: u64,
    bytes: u64,
}
struct Counted {
    inner: MemDevice<Vec<u8>>,
    counts: Counts,
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
        self.counts.calls += 1;
        self.counts.bytes += buf.len() as u64;
        Ok(())
    }
}
fn image(entries: usize) -> Vec<u8> {
    let mut tree = Tree::new();
    for i in 0..entries {
        tree.insert(format!("entry-{i:04}"), Node::file(Content::empty()))
            .unwrap();
    }
    tree.insert(
        "payload",
        Node::file(Content::bytes(vec![0x5a; 1024 * 1024])),
    )
    .unwrap();
    let options = UdfOptions::default();
    let size = hadris_udf::plan(&tree, &options).unwrap().size();
    let mut dev = MemDevice::new(vec![0; size as usize], BlockSize::new(2048).unwrap());
    hadris_udf::sync::write(&mut dev, &tree, &options).unwrap();
    dev.into_inner()
}
fn mount(bytes: &[u8], block: u32) -> UdfFs<Counted> {
    UdfFs::mount(
        Counted {
            inner: MemDevice::new(bytes.to_vec(), BlockSize::new(block).unwrap()),
            counts: Counts::default(),
        },
        MountOptions::new(),
    )
    .unwrap()
}
#[derive(Clone, Copy)]
enum Op {
    Mount,
    List,
    Last,
    Miss,
    Stat,
    Read(usize),
    Scattered,
}
fn row(bytes: &[u8], block: u32, entries: usize, label: &str, op: Op, samples: usize) {
    let mut times = Vec::new();
    let mut counts = None;
    for sample in 0..=samples {
        let mut fs = mount(bytes, block);
        let node = match op {
            Op::Stat | Op::Read(_) | Op::Scattered => {
                Some(fs.lookup(fs.root(), Name::new("payload")).unwrap())
            }
            _ => None,
        };
        let mut buf = vec![
            0;
            match op {
                Op::Read(n) => n,
                _ => 4096,
            }
        ];
        let before = match op {
            Op::Mount => Counts::default(),
            _ => fs.device().counts,
        };
        let mount_dev = matches!(op, Op::Mount).then(|| Counted {
            inner: MemDevice::new(bytes.to_vec(), BlockSize::new(block).unwrap()),
            counts: Counts::default(),
        });
        let start = Instant::now();
        match op {
            Op::Mount => {
                fs = UdfFs::mount(mount_dev.unwrap(), MountOptions::new()).unwrap();
            }
            Op::List => {
                let mut cursor = DirCursor::START;
                let mut n = 0;
                while let Some(entry) = fs.readdir(fs.root(), cursor).unwrap() {
                    cursor = entry.next_cursor();
                    n += 1;
                    black_box(entry);
                }
                assert_eq!(n, entries + 1);
            }
            Op::Last => {
                black_box(
                    fs.lookup(fs.root(), Name::new(&format!("entry-{:04}", entries - 1)))
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
            Op::Read(_) => {
                let mut pos = 0;
                while pos < 1024 * 1024 {
                    let n = fs.read(node.unwrap(), pos, &mut buf).unwrap();
                    assert!(n > 0);
                    assert!(buf[..n].iter().all(|&b| b == 0x5a));
                    black_box(&buf[..n]);
                    pos += n as u64;
                }
            }
            Op::Scattered => {
                for i in 0..256 {
                    let offset = ((i * 103) % 256) * 4096;
                    assert_eq!(fs.read(node.unwrap(), offset, &mut buf).unwrap(), 4096);
                    assert!(buf.iter().all(|&b| b == 0x5a));
                    black_box(&buf);
                }
            }
        }
        let elapsed = start.elapsed().as_nanos();
        if sample > 0 {
            times.push(elapsed);
            let current = Counts {
                calls: fs.device().counts.calls - before.calls,
                bytes: fs.device().counts.bytes - before.bytes,
            };
            if let Some(previous) = counts {
                assert!(previous == current);
            }
            counts = Some(current);
        }
    }
    times.sort_unstable();
    let counts = counts.unwrap();
    println!(
        "{entries}/{block}/{label},{},{},{}",
        times[times.len() / 2],
        counts.calls,
        counts.bytes
    );
}
fn main() {
    let samples: usize =
        std::env::var("HADRIS_UDF_BENCH_SAMPLES").map_or(7, |s| s.parse().unwrap());
    assert!(samples > 0);
    let filter = std::env::var("HADRIS_UDF_BENCH_FILTER").unwrap_or_default();
    println!("case,median_ns,read_calls,read_bytes");
    let mut selected = 0;
    for entries in [32, 1000] {
        let bytes = image(entries);
        for block in [512, 2048] {
            for (label, op) in [
                ("mount", Op::Mount),
                ("list", Op::List),
                ("lookup-last", Op::Last),
                ("lookup-miss", Op::Miss),
                ("stat-100", Op::Stat),
                ("read-4k", Op::Read(4096)),
                ("read-64k", Op::Read(65536)),
                ("read-scattered", Op::Scattered),
            ] {
                if format!("{entries}/{block}/{label}").contains(&filter) {
                    row(&bytes, block, entries, label, op, samples);
                    selected += 1;
                }
            }
        }
    }
    assert!(selected > 0, "filter matched no cases");
}

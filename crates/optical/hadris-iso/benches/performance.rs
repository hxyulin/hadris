use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, DirCursor, MountOptions, Name, Node, Tree};
use hadris_iso::{IsoOptions, sync::IsoFs};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, sync::BlockDevice};
use std::{cell::Cell, hint::black_box, rc::Rc, time::Instant};

#[derive(Default)]
struct Counts {
    reads: Cell<u64>,
    bytes: Cell<u64>,
    writes: Cell<u64>,
    written: Cell<u64>,
    max: Cell<usize>,
}
impl Counts {
    fn reset(&self) {
        self.reads.set(0);
        self.bytes.set(0);
        self.writes.set(0);
        self.written.set(0);
        self.max.set(0);
    }
    fn get(&self) -> (u64, u64, u64, u64, usize) {
        (
            self.reads.get(),
            self.bytes.get(),
            self.writes.get(),
            self.written.get(),
            self.max.get(),
        )
    }
}
struct Counted<D> {
    inner: D,
    counts: Rc<Counts>,
}
impl<D: hadris_io::ErrorType> hadris_io::ErrorType for Counted<D> {
    type Error = D::Error;
}
impl<D: BlockDevice> BlockDevice for Counted<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }
    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
    fn max_block_count(&self) -> u64 {
        self.inner.max_block_count()
    }
    fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.read_blocks(first, buf)?;
        self.counts.reads.set(self.counts.reads.get() + 1);
        self.counts
            .bytes
            .set(self.counts.bytes.get() + buf.len() as u64);
        self.counts.max.set(self.counts.max.get().max(buf.len()));
        Ok(())
    }
    fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.write_blocks(first, buf)?;
        self.counts.writes.set(self.counts.writes.get() + 1);
        self.counts
            .written
            .set(self.counts.written.get() + buf.len() as u64);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.flush()
    }
}
fn bs(n: u32) -> BlockSize {
    BlockSize::new(n).unwrap()
}
fn image(tree: &Tree, opts: &IsoOptions) -> Vec<u8> {
    let size = hadris_iso::plan(tree, opts).unwrap().size();
    let mut out = MemDevice::new(vec![0; size as usize], bs(2048));
    hadris_iso::sync::write(&mut out, tree, opts).unwrap();
    out.into_inner()
}
fn flat(n: usize, hard: bool) -> Tree {
    let mut tree = Tree::new();
    for i in 0..n {
        let name = format!("f{i:04}.txt");
        tree.insert(&name, Node::file(Content::bytes([i as u8; 16])))
            .unwrap();
        if hard {
            tree.link(&name, format!("l{i:04}.txt")).unwrap();
        }
    }
    tree
}
#[derive(Clone, Copy)]
enum Op {
    Mount,
    List(usize),
    Lookup(usize),
    Miss,
    All(usize),
    Repeat(usize),
    Stat,
    Read(usize, bool),
}
fn operate<D: BlockDevice>(
    fs: &mut IsoFs<D>,
    op: Op,
    rr: bool,
    buf: &mut [u8],
    node: Option<hadris_fs::NodeId>,
) {
    let root = fs.root();
    let lookup = |fs: &mut IsoFs<D>, i: usize| {
        let name = if rr {
            format!("f{i:04}.txt")
        } else {
            format!("F{i:04}.TXT")
        };
        black_box(fs.lookup(root, Name::new(name.as_bytes())).unwrap());
    };
    match op {
        Op::Mount => {}
        Op::List(expected) => {
            let mut cursor = DirCursor::START;
            let mut found = 0;
            while let Some(entry) = fs.readdir(root, cursor).unwrap() {
                cursor = entry.next_cursor();
                found += 1;
                black_box(entry);
            }
            assert_eq!(found, expected);
        }
        Op::Lookup(i) => lookup(fs, i),
        Op::Miss => assert!(fs.lookup(root, Name::new("absent")).is_err()),
        Op::All(n) => {
            for i in 0..n {
                lookup(fs, i);
            }
        }
        Op::Repeat(i) => {
            for _ in 0..100 {
                lookup(fs, i);
            }
        }
        Op::Stat => {
            for _ in 0..100 {
                black_box(fs.stat(node.unwrap()).unwrap());
            }
        }
        Op::Read(_, unaligned) => {
            let mut pos = if unaligned { 1 } else { 0 };
            while pos < 1024 * 1024 {
                let n = fs.read(node.unwrap(), pos, buf).unwrap();
                assert!(n > 0);
                black_box(&buf[..n]);
                pos += n as u64;
            }
        }
    }
}
fn row<D: BlockDevice>(
    label: &str,
    bytes: &[u8],
    block: u32,
    rr: bool,
    op: Op,
    make: impl Fn(Counted<MemDevice<Vec<u8>>>) -> D,
    cache: u8,
) {
    if std::env::var("HADRIS_ISO_BENCH_FILTER").is_ok_and(|f| !label.contains(&f)) {
        return;
    }
    let samples = samples();
    let mut times = Vec::new();
    let mut expected = None;
    let mut logical_expected = None;
    for sample in 0..=samples {
        let physical = Rc::new(Counts::default());
        let logical = Rc::new(Counts::default());
        let dev = Counted {
            inner: make(Counted {
                inner: MemDevice::new(bytes.to_vec(), bs(block)),
                counts: physical.clone(),
            }),
            counts: logical.clone(),
        };
        let mut buf = vec![
            0;
            if let Op::Read(chunk, _) = op {
                chunk
            } else {
                0
            }
        ];
        let start;
        let mut fs;
        let mut node = None;
        if matches!(op, Op::Mount) {
            start = Instant::now();
            fs = configure(IsoFs::mount(dev, MountOptions::new()).unwrap(), cache);
        } else {
            fs = configure(IsoFs::mount(dev, MountOptions::new()).unwrap(), cache);
            if matches!(op, Op::Stat | Op::Read(..)) {
                node = Some(
                    fs.lookup(
                        fs.root(),
                        Name::new(if matches!(op, Op::Stat) {
                            if rr { "f0000.txt" } else { "F0000.TXT" }
                        } else {
                            "BIG.BIN"
                        }),
                    )
                    .unwrap(),
                );
            }
            physical.reset();
            logical.reset();
            start = Instant::now();
        }
        operate(&mut fs, op, rr, &mut buf, node);
        let elapsed = start.elapsed().as_nanos();
        if let Op::Read(chunk, unaligned) = op {
            let remainder = (1024 * 1024 - usize::from(unaligned)) % chunk;
            let valid = if remainder == 0 { chunk } else { remainder };
            assert!(buf[..valid].iter().all(|&byte| byte == 42));
        }
        if let Some(e) = expected {
            assert_eq!(e, physical.get(), "{label}");
            assert_eq!(logical_expected.unwrap(), logical.get(), "{label}");
        } else {
            expected = Some(physical.get());
            logical_expected = Some(logical.get());
        }
        if sample != 0 {
            times.push(elapsed);
        }
    }
    times.sort();
    let (reads, bytes, writes, written, max) = expected.unwrap();
    let (logical, logical_bytes, ..) = logical_expected.unwrap();
    println!(
        "{label},{block},{logical},{logical_bytes},{reads},{bytes},{max},{writes},{written},{},{},{}",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}
fn writer(label: &str, tree: &Tree, opts: &IsoOptions, write: bool) {
    if std::env::var("HADRIS_ISO_BENCH_FILTER").is_ok_and(|f| !label.contains(&f)) {
        return;
    }
    let size = hadris_iso::plan(tree, opts).unwrap().size();
    let samples = samples();
    let mut times = Vec::new();
    let mut counts = None;
    for sample in 0..=samples {
        let c = Rc::new(Counts::default());
        let mut out = Counted {
            inner: MemDevice::new(vec![0; size as usize], bs(2048)),
            counts: c.clone(),
        };
        let start = Instant::now();
        if write {
            black_box(hadris_iso::sync::write(&mut out, tree, opts).unwrap());
        } else {
            black_box(hadris_iso::plan(tree, opts).unwrap());
        }
        let elapsed = start.elapsed().as_nanos();
        if let Some(expected) = counts {
            assert_eq!(expected, c.get());
        } else {
            counts = Some(c.get());
        }
        if sample != 0 {
            times.push(elapsed);
        }
    }
    times.sort();
    let (r, b, w, wb, max) = counts.unwrap();
    println!(
        "{label},2048,{r},{b},{r},{b},{max},{w},{wb},{},{},{}",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}
fn samples() -> usize {
    std::env::var("HADRIS_ISO_BENCH_SAMPLES")
        .ok()
        .map(|v| {
            v.parse::<usize>()
                .expect("samples must be a positive integer")
        })
        .unwrap_or(7)
        .max(1)
}
fn configure<D: BlockDevice>(fs: IsoFs<D>, mode: u8) -> IsoFs<D> {
    #[cfg(feature = "cache")]
    if mode != 0 {
        let options = hadris_iso::CacheOptions::new()
            .with_blocks(64)
            .with_records(128)
            .with_links(if mode == 2 { 1024 } else { 0 });
        return fs.with_cache(options);
    }
    let _ = mode;
    fs
}
fn main() {
    println!(
        "case,device_block,logical_reads,logical_bytes,backing_reads,backing_bytes,max_read,write_calls,write_bytes,min_ns,median_ns,max_ns"
    );
    for n in [32, 128, 512] {
        for (kind, rr, hard) in [
            ("primary", false, false),
            ("rr", true, false),
            ("links", true, true),
        ] {
            let opts = if rr {
                IsoOptions::new().with_rock_ridge()
            } else {
                IsoOptions::new()
            };
            let bytes = image(&flat(n, hard), &opts);
            let modes: &[u8] = if cfg!(feature = "cache") {
                &[0, 1, 2]
            } else {
                &[0]
            };
            for &mode in modes {
                for (name, op) in [
                    ("mount", Op::Mount),
                    ("first", Op::Lookup(0)),
                    ("last", Op::Lookup(n - 1)),
                    ("miss", Op::Miss),
                    ("all", Op::All(n)),
                    ("list", Op::List(n * if hard { 2 } else { 1 })),
                    ("repeat-last100", Op::Repeat(n - 1)),
                    ("stat100", Op::Stat),
                ] {
                    row(
                        &format!("{kind}-{n}-cache{mode}-{name}"),
                        &bytes,
                        2048,
                        rr,
                        op,
                        |dev| dev,
                        mode,
                    );
                }
            }
        }
        let mut tree = Tree::new();
        for i in 0..n {
            tree.insert(
                format!("dir{i:04}/file.txt"),
                Node::file(Content::bytes("payload")),
            )
            .unwrap();
        }
        for (kind, opts) in [
            ("primary", IsoOptions::new()),
            ("rr", IsoOptions::new().with_rock_ridge()),
            (
                "three",
                IsoOptions::new()
                    .with_rock_ridge()
                    .with_joliet()
                    .with_iso1999(),
            ),
        ] {
            writer(&format!("plan-{kind}-dirs{}", n + 1), &tree, &opts, false);
            writer(&format!("write-{kind}-dirs{}", n + 1), &tree, &opts, true);
        }
    }
    let mut tree = Tree::new();
    tree.insert("big.bin", Node::file(Content::bytes(vec![42; 1024 * 1024])))
        .unwrap();
    let bytes = image(&tree, &IsoOptions::new());
    for block in [512, 2048, 4096] {
        for chunk in [512, 4096, 65536, 1024 * 1024] {
            for unaligned in [false, true] {
                let modes: &[u8] = if cfg!(feature = "cache") {
                    &[0, 1]
                } else {
                    &[0]
                };
                for &mode in modes {
                    row(
                        &format!("read-1MiB-cache{mode}-chunk{chunk}-unaligned{unaligned}"),
                        &bytes,
                        block,
                        false,
                        Op::Read(chunk, unaligned),
                        |dev| dev,
                        mode,
                    );
                }
            }
        }
    }
}

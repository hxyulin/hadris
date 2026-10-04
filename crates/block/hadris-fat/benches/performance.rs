mod support;

use std::cell::Cell;
use std::convert::Infallible;
use std::hint::black_box;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use hadris_fat::embedded::sync::Fat;
use hadris_fat::embedded::{File, MountToken};
use hadris_fat::sync::{FatFs, check, format};
use hadris_fat::{CacheOptions, FatKind, FatOptions};
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, MountOptions, Name, NodeId, OpenMode, OpenOptions, SeekFrom, SetAttr};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockSize, MemDevice};
use support::{Counted, IoCounts, WriteBreakdown};

const BULK_BYTES: usize = 1 << 20;
const LOG_BYTES: usize = 16 << 10;
const DIRECTORY_FILES: usize = 128;
const TEMP_FILES: usize = 64;
const FILE_NAME: &str = "DATA.BIN";
const BOOT_NAME: &str = "EFI/BOOT/BOOTAA64.EFI";

trait Driver {
    type Handle;
    fn open_file(&mut self, name: &str, create: bool) -> Self::Handle;
    fn close_file(&mut self, file: Self::Handle);
    fn seek_file(&mut self, file: &Self::Handle, offset: usize);
    fn read_file(&mut self, file: &Self::Handle, offset: usize, buf: &mut [u8]) -> usize;
    fn write_file(&mut self, file: &Self::Handle, offset: usize, buf: &[u8]) -> usize;
    fn flush_file(&mut self, file: &Self::Handle);
    fn remove_file(&mut self, name: &str);
    fn lookup_file(&mut self, name: &str);
    fn list_files(&mut self) -> usize;
    fn sync_volume(&mut self);
}

impl<D: BlockDevice<Error = Infallible>> Driver for FatFs<D> {
    type Handle = NodeId;

    fn open_file(&mut self, name: &str, create: bool) -> NodeId {
        let mut root = self.root();
        let mut parts = name.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            let next = self.lookup(root, Name::new(part)).unwrap();
            self.forget(root, 1);
            root = next;
        }
        let name = name.rsplit('/').next().unwrap();
        let file = if create {
            self.create(root, Name::new(name), &SetAttr::new()).unwrap()
        } else {
            self.lookup(root, Name::new(name)).unwrap()
        };
        self.forget(root, 1);
        self.open(
            file,
            if create {
                OpenMode::Write
            } else {
                OpenMode::Read
            },
        )
        .unwrap();
        file
    }
    fn close_file(&mut self, file: NodeId) {
        self.close(file).unwrap();
        self.forget(file, 1);
    }
    fn seek_file(&mut self, _: &NodeId, _: usize) {}
    fn read_file(&mut self, file: &NodeId, offset: usize, buf: &mut [u8]) -> usize {
        self.read(*file, offset as u64, buf).unwrap()
    }
    fn write_file(&mut self, file: &NodeId, offset: usize, buf: &[u8]) -> usize {
        self.write(*file, offset as u64, buf).unwrap()
    }
    fn flush_file(&mut self, file: &NodeId) {
        self.fsync(*file).unwrap();
    }
    fn remove_file(&mut self, name: &str) {
        self.unlink(self.root(), Name::new(name)).unwrap();
    }
    fn lookup_file(&mut self, name: &str) {
        let file = self.lookup(self.root(), Name::new(name)).unwrap();
        black_box(self.stat(file).unwrap());
        self.forget(file, 1);
    }
    fn list_files(&mut self) -> usize {
        let mut cursor = DirCursor::START;
        let mut count = 0;
        while let Some(entry) = self.readdir(self.root(), cursor).unwrap() {
            cursor = entry.next_cursor();
            black_box(&entry);
            count += 1;
        }
        count
    }
    fn sync_volume(&mut self) {
        self.sync().unwrap();
    }
}

impl<'mount, D: BlockDevice<Error = Infallible>> Driver for Fat<'mount, D> {
    type Handle = File<'mount>;

    fn open_file(&mut self, name: &str, create: bool) -> File<'mount> {
        let options = if create {
            OpenOptions::new().write().create()
        } else {
            OpenOptions::new().read()
        };
        let mut dir = self.root();
        let mut parts = name.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return self.open(dir, part, options).unwrap();
            }
            dir = self.open_dir(dir, part).unwrap();
        }
        unreachable!()
    }
    fn close_file(&mut self, file: File<'mount>) {
        self.close(file).unwrap();
    }
    fn seek_file(&mut self, file: &File<'mount>, offset: usize) {
        self.seek(file, SeekFrom::Start(offset as u64)).unwrap();
    }
    fn read_file(&mut self, file: &File<'mount>, _: usize, buf: &mut [u8]) -> usize {
        self.read(file, buf).unwrap()
    }
    fn write_file(&mut self, file: &File<'mount>, _: usize, buf: &[u8]) -> usize {
        self.write(file, buf).unwrap()
    }
    fn flush_file(&mut self, file: &File<'mount>) {
        self.flush(file).unwrap();
    }
    fn remove_file(&mut self, name: &str) {
        self.remove_file(self.root(), name).unwrap();
    }
    fn lookup_file(&mut self, name: &str) {
        black_box(self.metadata(self.root(), name).unwrap());
    }
    fn list_files(&mut self) -> usize {
        let mut count = 0;
        self.list(self.root(), DirCursor::START, |entry| {
            black_box(entry);
            count += 1;
            ControlFlow::Continue(())
        })
        .unwrap();
        count
    }
    fn sync_volume(&mut self) {
        self.sync().unwrap();
    }
}

#[derive(Clone, Copy)]
enum Workload {
    Mount,
    Write {
        chunk: usize,
        bytes: usize,
        flush_each: bool,
    },
    Read {
        chunk: usize,
    },
    CreateRemove {
        long: bool,
    },
    ReadSeek {
        reverse: bool,
    },
    ReadFragmented,
    BootLoad,
    Lookup,
    LookupLong,
    List,
    ListLong,
}

const WORKLOADS: [(&str, Workload); 21] = [
    ("mount", Workload::Mount),
    (
        "append-64-end-sync",
        Workload::Write {
            chunk: 64,
            bytes: LOG_BYTES,
            flush_each: false,
        },
    ),
    (
        "append-64-flush-each",
        Workload::Write {
            chunk: 64,
            bytes: LOG_BYTES,
            flush_each: true,
        },
    ),
    (
        "append-512-end-sync",
        Workload::Write {
            chunk: 512,
            bytes: LOG_BYTES,
            flush_each: false,
        },
    ),
    (
        "append-4k-end-sync",
        Workload::Write {
            chunk: 4096,
            bytes: LOG_BYTES,
            flush_each: false,
        },
    ),
    (
        "write-4k",
        Workload::Write {
            chunk: 4096,
            bytes: BULK_BYTES,
            flush_each: false,
        },
    ),
    (
        "write-64k",
        Workload::Write {
            chunk: 65536,
            bytes: BULK_BYTES,
            flush_each: false,
        },
    ),
    ("read-128", Workload::Read { chunk: 128 }),
    ("read-4k", Workload::Read { chunk: 4096 }),
    ("read-64k", Workload::Read { chunk: 65536 }),
    ("read-unaligned", Workload::Read { chunk: 4093 }),
    ("read-reverse-4k", Workload::ReadSeek { reverse: true }),
    ("read-shuffled-4k", Workload::ReadSeek { reverse: false }),
    ("read-fragmented-64k", Workload::ReadFragmented),
    ("boot-load-64k", Workload::BootLoad),
    ("lookup-long-128", Workload::LookupLong),
    ("list-long-128", Workload::ListLong),
    (
        "create-remove-short",
        Workload::CreateRemove { long: false },
    ),
    ("create-remove-long", Workload::CreateRemove { long: true }),
    ("lookup-128", Workload::Lookup),
    ("list-128", Workload::List),
];

impl Workload {
    fn payload_bytes(self) -> usize {
        match self {
            Self::Write { bytes, .. } => bytes,
            Self::Read { .. } | Self::ReadSeek { .. } | Self::ReadFragmented | Self::BootLoad => {
                BULK_BYTES
            }
            Self::CreateRemove { .. } => TEMP_FILES * 64,
            _ => 0,
        }
    }
}

struct Inputs {
    payload: Vec<u8>,
    short_names: Vec<String>,
    long_names: Vec<String>,
}

impl Inputs {
    fn new() -> Self {
        Self {
            payload: (0..BULK_BYTES)
                .map(|i| {
                    let mut value = (i as u32).wrapping_add(0x9e37_79b9);
                    value = (value ^ (value >> 16)).wrapping_mul(0x85eb_ca6b);
                    value = (value ^ (value >> 13)).wrapping_mul(0xc2b2_ae35);
                    (value ^ (value >> 16)) as u8
                })
                .collect(),
            short_names: (0..DIRECTORY_FILES)
                .map(|i| format!("F{i:07}.BIN"))
                .collect(),
            long_names: (0..DIRECTORY_FILES)
                .map(|i| format!("File {i:04} with a long name.bin"))
                .collect(),
        }
    }
}

fn execute<F: Driver>(
    fs: &mut F,
    workload: Workload,
    input: &Inputs,
    buf: &mut [u8],
    verify: bool,
) {
    match workload {
        Workload::Mount => unreachable!(),
        Workload::Write {
            chunk,
            bytes,
            flush_each,
        } => {
            let file = fs.open_file(FILE_NAME, true);
            let mut offset = 0;
            while offset < bytes {
                let end = (offset + chunk).min(bytes);
                let n = fs.write_file(&file, offset, &input.payload[offset..end]);
                assert!(n > 0 && n <= end - offset);
                offset += n;
                if flush_each {
                    fs.flush_file(&file);
                }
            }
            fs.close_file(file);
            fs.sync_volume();
        }
        Workload::Read { .. } | Workload::ReadFragmented | Workload::BootLoad => {
            let chunk = match workload {
                Workload::Read { chunk } => chunk,
                _ => 65536,
            };
            let name = if matches!(workload, Workload::BootLoad) {
                BOOT_NAME
            } else {
                FILE_NAME
            };
            let file = fs.open_file(name, false);
            let mut offset = 0;
            loop {
                let n = fs.read_file(&file, offset, &mut buf[..chunk]);
                if n == 0 {
                    break;
                }
                assert!(offset + n <= BULK_BYTES);
                if verify {
                    assert_eq!(&buf[..n], &input.payload[offset..offset + n]);
                }
                black_box(&buf[..n]);
                offset += n;
            }
            assert_eq!(offset, BULK_BYTES);
            fs.close_file(file);
        }
        Workload::ReadSeek { reverse } => {
            let file = fs.open_file(FILE_NAME, false);
            let blocks = BULK_BYTES / 4096;
            for i in 0..blocks {
                let block = if reverse {
                    blocks - 1 - i
                } else {
                    (i * 73) % blocks
                };
                let offset = block * 4096;
                fs.seek_file(&file, offset);
                let n = fs.read_file(&file, offset, &mut buf[..4096]);
                assert_eq!(n, 4096);
                if verify {
                    assert_eq!(&buf[..n], &input.payload[offset..offset + n]);
                }
                black_box(&buf[..n]);
            }
            fs.close_file(file);
        }
        Workload::CreateRemove { long } => {
            let names = if long {
                &input.long_names
            } else {
                &input.short_names
            };
            for name in &names[..TEMP_FILES] {
                let file = fs.open_file(name, true);
                let mut offset = 0;
                while offset < 64 {
                    let n = fs.write_file(&file, offset, &input.payload[offset..64]);
                    assert!(n > 0 && n <= 64 - offset);
                    offset += n;
                }
                fs.close_file(file);
            }
            for name in &names[..TEMP_FILES] {
                fs.remove_file(name);
            }
            fs.sync_volume();
        }
        Workload::Lookup | Workload::LookupLong => {
            let names = if matches!(workload, Workload::LookupLong) {
                &input.long_names
            } else {
                &input.short_names
            };
            for name in names {
                fs.lookup_file(name);
            }
        }
        Workload::List | Workload::ListLong => assert_eq!(fs.list_files(), DIRECTORY_FILES),
    }
}

struct Sample {
    elapsed: Duration,
    counts: IoCounts,
    image: Vec<u8>,
    writes: WriteBreakdown,
}

fn sample(
    image: &[u8],
    embedded: bool,
    workload: Workload,
    input: &Inputs,
    verify: bool,
    positions: usize,
    blocks: usize,
) -> Sample {
    let counts = Cell::new(IoCounts::default());
    let written_blocks: Vec<Cell<u64>> = if verify {
        vec![Cell::new(0); image.len() / 512]
    } else {
        Vec::new()
    };
    let dev = Counted {
        inner: MemDevice::new(image.to_vec(), BlockSize::new(512).unwrap()),
        counts: &counts,
        written_blocks: verify.then_some(written_blocks.as_slice()),
    };
    let mut buf = vec![0; 65536];
    let mut token = MountToken::new();
    let mount_start = Instant::now();
    let (elapsed, dev) = if embedded {
        let mut fs = Fat::mount(dev, &mut token).unwrap();
        if matches!(workload, Workload::Mount) {
            (mount_start.elapsed(), fs.into_inner())
        } else if matches!(workload, Workload::BootLoad) {
            execute(&mut fs, workload, input, &mut buf, verify);
            (mount_start.elapsed(), fs.into_inner())
        } else {
            counts.set(IoCounts::default());
            for count in &written_blocks {
                count.set(0);
            }
            let start = Instant::now();
            execute(&mut fs, workload, input, &mut buf, verify);
            (start.elapsed(), fs.into_inner())
        }
    } else {
        let options = if matches!(workload, Workload::BootLoad) {
            MountOptions::new().read_only()
        } else {
            MountOptions::new()
        };
        let mut fs = FatFs::mount(dev, options).unwrap().with_cache(
            CacheOptions::new()
                .with_chain_positions(positions)
                .with_blocks(blocks),
        );
        if matches!(workload, Workload::Mount) {
            (mount_start.elapsed(), fs.into_inner())
        } else if matches!(workload, Workload::BootLoad) {
            execute(&mut fs, workload, input, &mut buf, verify);
            (mount_start.elapsed(), fs.into_inner())
        } else {
            counts.set(IoCounts::default());
            for count in &written_blocks {
                count.set(0);
            }
            let start = Instant::now();
            execute(&mut fs, workload, input, &mut buf, verify);
            (start.elapsed(), fs.into_inner())
        }
    };
    let image = dev.inner.into_inner();
    let writes = if verify {
        WriteBreakdown::classify(&image, &written_blocks)
    } else {
        WriteBreakdown::default()
    };
    if verify {
        assert_eq!(writes.total(), counts.get().write_bytes);
    }
    Sample {
        elapsed,
        counts: counts.get(),
        image,
        writes,
    }
}

fn fixture(blank: &[u8], workload: Workload, input: &Inputs) -> Vec<u8> {
    if !matches!(
        workload,
        Workload::Read { .. }
            | Workload::ReadSeek { .. }
            | Workload::ReadFragmented
            | Workload::BootLoad
            | Workload::Lookup
            | Workload::LookupLong
            | Workload::List
            | Workload::ListLong
    ) {
        return blank.to_vec();
    }
    let dev = MemDevice::new(blank.to_vec(), BlockSize::new(512).unwrap());
    let mut fs = FatFs::mount(dev, MountOptions::new()).unwrap();
    if matches!(
        workload,
        Workload::Read { .. }
            | Workload::ReadSeek { .. }
            | Workload::ReadFragmented
            | Workload::BootLoad
    ) {
        let name = if matches!(workload, Workload::BootLoad) {
            let root = fs.root();
            let efi = fs.mkdir(root, Name::new("EFI"), &SetAttr::new()).unwrap();
            let boot = fs.mkdir(efi, Name::new("BOOT"), &SetAttr::new()).unwrap();
            fs.forget(efi, 1);
            fs.forget(boot, 1);
            BOOT_NAME
        } else {
            FILE_NAME
        };
        let file = fs.open_file(name, true);
        assert_eq!(fs.write_file(&file, 0, &input.payload), BULK_BYTES);
        fs.close_file(file);
    } else {
        let names = if matches!(workload, Workload::LookupLong | Workload::ListLong) {
            &input.long_names
        } else {
            &input.short_names
        };
        for name in names {
            let file = fs.open_file(name, true);
            fs.close_file(file);
        }
    }
    let mut image = fs.unmount().unwrap().into_inner();
    if matches!(workload, Workload::ReadFragmented) {
        fragment(&mut image);
    }
    image
}

fn fragment(image: &mut [u8]) {
    let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
    let kind = geo.kind();
    let first = {
        let dev = MemDevice::new(&*image, BlockSize::new(512).unwrap());
        let mut fs = FatFs::mount(dev, MountOptions::new().read_only()).unwrap();
        let file = fs.lookup(fs.root(), Name::new(FILE_NAME)).unwrap();
        let mut runs = [hadris_fs::Extent::new(0, 0)];
        assert_eq!(fs.extents(file, 0, &mut runs).unwrap(), 1);
        ((runs[0].offset() - geo.data_start()) / geo.cluster_size() as u64 + 2) as u32
    };
    let mut chain = Vec::new();
    let mut cluster = first;
    loop {
        chain.push(cluster);
        let at = (geo.fat_copy(geo.active_fat()) + kind.entry_offset(cluster as u64)) as usize;
        match kind
            .next(
                kind.decode(cluster as u64, &image[at..at + kind.entry_len()]),
                geo.max_cluster(),
            )
            .unwrap()
        {
            Some(next) => cluster = next,
            None => break,
        }
    }
    let order: Vec<_> = chain
        .iter()
        .step_by(2)
        .chain(chain.iter().skip(1).step_by(2))
        .copied()
        .collect();
    assert_eq!(order[0], first);
    let original = image.to_vec();
    for (i, &cluster) in order.iter().enumerate() {
        let source = geo.cluster_offset(chain[i]).unwrap() as usize;
        let target = geo.cluster_offset(cluster).unwrap() as usize;
        let bytes = geo.cluster_size() as usize;
        image[target..target + bytes].copy_from_slice(&original[source..source + bytes]);
        let next = order.get(i + 1).copied().unwrap_or(kind.end_of_chain());
        for copy in 0..geo.fat_count() {
            let at = (geo.fat_copy(copy) + kind.entry_offset(cluster as u64)) as usize;
            kind.encode(cluster as u64, next, &mut image[at..at + kind.entry_len()]);
        }
    }
}

fn validate(image: &[u8], workload: Workload, input: &Inputs) {
    let mut dev = MemDevice::new(image, BlockSize::new(512).unwrap());
    let report = check(&mut dev, &mut vec![0; 16384], |finding| {
        eprintln!("{finding}")
    })
    .unwrap();
    assert!(report.is_clean(), "benchmark left a damaged filesystem");
    let mut fs = FatFs::mount(dev, MountOptions::new().read_only()).unwrap();
    match workload {
        Workload::Write { .. }
        | Workload::Read { .. }
        | Workload::ReadSeek { .. }
        | Workload::ReadFragmented
        | Workload::BootLoad => {
            let bytes = workload.payload_bytes();
            let name = if matches!(workload, Workload::BootLoad) {
                BOOT_NAME
            } else {
                FILE_NAME
            };
            let file = fs.open_file(name, false);
            assert_eq!(fs.stat(file).unwrap().len(), bytes as u64);
            let mut buf = vec![0; 65536];
            let mut offset = 0;
            while offset < bytes {
                let n = fs.read_file(&file, offset, &mut buf);
                assert!(n > 0 && offset + n <= bytes);
                assert_eq!(&buf[..n], &input.payload[offset..offset + n]);
                offset += n;
            }
            fs.close_file(file);
        }
        Workload::Lookup | Workload::LookupLong | Workload::List | Workload::ListLong => {
            assert_eq!(fs.list_files(), DIRECTORY_FILES)
        }
        Workload::CreateRemove { .. } | Workload::Mount => assert_eq!(fs.list_files(), 0),
    }
}

struct Options {
    samples: usize,
    filter: String,
    csv: bool,
    positions: usize,
    blocks: usize,
}

fn options() -> Options {
    let mut options = Options {
        samples: 7,
        filter: String::new(),
        csv: false,
        positions: 0,
        blocks: 0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => {
                options.samples = args
                    .next()
                    .expect("--samples needs a positive integer")
                    .parse()
                    .expect("invalid sample count");
                assert!(options.samples > 0, "sample count must be positive");
            }
            "--filter" => options.filter = args.next().expect("--filter needs a substring"),
            "--chain-positions" => {
                options.positions = args
                    .next()
                    .expect("--chain-positions needs an integer")
                    .parse()
                    .expect("invalid chain-position count")
            }
            "--metadata-blocks" => {
                options.blocks = args
                    .next()
                    .expect("--metadata-blocks needs an integer")
                    .parse()
                    .expect("invalid block count")
            }
            "--csv" => options.csv = true,
            "--smoke" => options.samples = 1,
            "--bench" => {}
            "--help" | "-h" => {
                println!(
                    "performance [--samples N] [--filter SUBSTRING] [--csv] [--smoke] [--chain-positions N] [--metadata-blocks N]\nCase names: fat12|fat16|fat32/hosted|embedded/<workload>"
                );
                std::process::exit(0);
            }
            _ => panic!("unknown argument {arg}; use --help"),
        }
    }
    options
}

fn main() {
    let options = options();
    let input = Inputs::new();
    if options.csv {
        println!(
            "case,samples,volume_bytes,block_bytes,cluster_bytes,min_ns,median_ns,max_ns,payload_bytes,read_calls,write_calls,read_bytes,write_bytes,flush_calls,max_read_bytes,max_write_bytes,write_amplification,fat0_write_bytes,fat1_write_bytes,directory_write_bytes,data_write_bytes,fsinfo_write_bytes,other_write_bytes,chain_positions,metadata_blocks"
        );
    } else {
        type StateDevice = MemDevice<&'static mut [u8]>;
        eprintln!(
            "In-memory FAT baseline; successful device I/O counts per workload; setup and validation excluded."
        );
        eprintln!(
            "Host driver state bytes excluding device (heap and stack excluded): hosted={}, embedded FILES=1:{}, 4:{}, 16:{}",
            size_of::<FatFs<StateDevice>>() - size_of::<StateDevice>(),
            size_of::<Fat<'static, StateDevice, 1>>() - size_of::<StateDevice>(),
            size_of::<Fat<'static, StateDevice, 4>>() - size_of::<StateDevice>(),
            size_of::<Fat<'static, StateDevice, 16>>() - size_of::<StateDevice>()
        );
        println!(
            "{:<43} {:>10} {:>10} {:>10} {:>12} {:>12} {:>8} {:>8}",
            "case",
            "median us",
            "reads",
            "writes",
            "read bytes",
            "write bytes",
            "flushes",
            "write x"
        );
    }
    let mut selected = 0;
    for (kind_name, kind, size) in [
        ("fat12", FatKind::Fat12, 2 << 20),
        ("fat16", FatKind::Fat16, 16 << 20),
        ("fat32", FatKind::Fat32, 64 << 20),
    ] {
        if !["hosted", "embedded"].iter().any(|backend| {
            WORKLOADS
                .iter()
                .any(|(name, _)| format!("{kind_name}/{backend}/{name}").contains(&options.filter))
        }) {
            continue;
        }
        let mut dev = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        let geometry = format(&mut dev, &FatOptions::new().with_kind(kind).with_serial(1)).unwrap();
        let cluster_bytes = geometry.cluster_size();
        if !options.csv {
            eprintln!(
                "{kind_name}: {size} bytes, 512-byte blocks, {cluster_bytes}-byte clusters, two FAT copies"
            );
        }
        let blank = dev.into_inner();
        for (name, workload) in WORKLOADS {
            for embedded in [false, true] {
                let case = format!(
                    "{kind_name}/{}/{name}",
                    if embedded { "embedded" } else { "hosted" }
                );
                if !case.contains(&options.filter) {
                    continue;
                }
                selected += 1;
                let image = fixture(&blank, workload, &input);
                let warmup = sample(
                    &image,
                    embedded,
                    workload,
                    &input,
                    true,
                    options.positions,
                    options.blocks,
                );
                validate(&warmup.image, workload, &input);
                let counts = warmup.counts;
                let writes = warmup.writes;
                drop(warmup);
                let mut timings = Vec::with_capacity(options.samples);
                for _ in 0..options.samples {
                    let result = sample(
                        &image,
                        embedded,
                        workload,
                        &input,
                        false,
                        options.positions,
                        options.blocks,
                    );
                    assert_eq!(
                        result.counts, counts,
                        "non-deterministic device I/O in {case}"
                    );
                    timings.push(result.elapsed.as_nanos());
                }
                timings.sort_unstable();
                let median = timings[timings.len() / 2];
                let payload = workload.payload_bytes();
                let amplification = if matches!(
                    workload,
                    Workload::Write { .. } | Workload::CreateRemove { .. }
                ) {
                    format!("{:.3}", counts.write_bytes as f64 / payload as f64)
                } else {
                    String::from("n/a")
                };
                if options.csv {
                    println!(
                        "{case},{},{size},512,{cluster_bytes},{},{median},{},{payload},{},{},{},{},{},{},{},{amplification},{},{},{},{},{},{},{},{}",
                        options.samples,
                        timings[0],
                        timings[timings.len() - 1],
                        counts.read_calls,
                        counts.write_calls,
                        counts.read_bytes,
                        counts.write_bytes,
                        counts.flush_calls,
                        counts.max_read_bytes,
                        counts.max_write_bytes,
                        writes.fat_bytes[0],
                        writes.fat_bytes[1],
                        writes.directory_bytes,
                        writes.data_bytes,
                        writes.fs_info_bytes,
                        writes.other_bytes,
                        if embedded { 0 } else { options.positions },
                        if embedded { 0 } else { options.blocks }
                    );
                } else {
                    println!(
                        "{case:<43} {:>10.2} {:>10} {:>10} {:>12} {:>12} {:>8} {amplification:>8}",
                        median as f64 / 1000.0,
                        counts.read_calls,
                        counts.write_calls,
                        counts.read_bytes,
                        counts.write_bytes,
                        counts.flush_calls
                    );
                }
            }
        }
    }
    assert!(selected > 0, "filter matched no benchmark cases");
}

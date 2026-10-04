use hadris_cpio::sync::{CpioReader, read_tree, write};
use hadris_cpio::{CpioOptions, Format};
use hadris_fs::{Content, Node, Tree};
use hadris_io::{StdIo, sync::Read};
use std::cell::Cell;
use std::hint::black_box;
use std::io::{BufReader, BufWriter};
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    reads: u64,
    read_bytes: u64,
    writes: u64,
    write_bytes: u64,
    flushes: u64,
    max_read: usize,
    max_write: usize,
}

struct Counted<T> {
    inner: T,
    counts: Rc<Cell<Counts>>,
}

impl<T: std::io::Read> std::io::Read for Counted<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let mut counts = self.counts.get();
        counts.reads += 1;
        counts.read_bytes += n as u64;
        counts.max_read = counts.max_read.max(buf.len());
        self.counts.set(counts);
        Ok(n)
    }
}

impl<T: std::io::Write> std::io::Write for Counted<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(black_box(buf))?;
        let mut counts = self.counts.get();
        counts.writes += 1;
        counts.write_bytes += n as u64;
        counts.max_write = counts.max_write.max(buf.len());
        self.counts.set(counts);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()?;
        let mut counts = self.counts.get();
        counts.flushes += 1;
        self.counts.set(counts);
        Ok(())
    }
}

fn fixture(workload: &str) -> (Tree, Option<tempfile::NamedTempFile>) {
    let mut tree = Tree::new();
    let mut file = None;
    match workload {
        "stream-large" | "stream-small" => {
            let len = if workload == "stream-large" {
                4 * 1024 * 1024
            } else {
                17
            };
            let mut source = tempfile::NamedTempFile::new().unwrap();
            std::io::Write::write_all(&mut source, &vec![37; len]).unwrap();
            tree.insert(
                "data",
                Node::file(hadris_fs::host::file(source.path()).unwrap()),
            )
            .unwrap();
            file = Some(source);
        }
        "large" => {
            let data = (0..4 * 1024 * 1024)
                .map(|i| (i * 37) as u8)
                .collect::<Vec<_>>();
            tree.insert("data", Node::file(Content::bytes(data)))
                .unwrap();
        }
        "links" => {
            tree.insert("f0000", Node::file(Content::bytes(vec![17; 4097])))
                .unwrap();
            for i in 1..256 {
                tree.link("f0000", format!("f{i:04}")).unwrap();
            }
        }
        "metadata" => {
            for i in 0..128 {
                tree.insert(format!("d{i:04}"), Node::dir()).unwrap();
                tree.insert(format!("s{i:04}"), Node::symlink("d0000"))
                    .unwrap();
            }
        }
        "small" => {
            for i in 0..256 {
                tree.insert(
                    format!("f{i:04}"),
                    Node::file(Content::bytes(vec![i as u8; 17])),
                )
                .unwrap();
            }
        }
        _ => unreachable!(),
    }
    (tree, file)
}

fn check_tree(tree: &Tree, expected: &Tree) {
    for (name, entry) in expected.root().children() {
        let got = tree.entry(name.as_bytes()).unwrap();
        assert_eq!(got.node().file_type(), entry.node().file_type());
        assert_eq!(got.node().target(), entry.node().target());
        if let Some(content) = entry.node().content() {
            let expected = if let Some(bytes) = content.as_bytes() {
                bytes.to_vec()
            } else {
                let mut reader = hadris_fs::sync::ContentReader::open(content).unwrap();
                let mut bytes = vec![0; content.len() as usize];
                reader.read_exact_at(0, &mut bytes).unwrap();
                bytes
            };
            assert_eq!(
                got.node().content().unwrap().as_bytes(),
                Some(expected.as_slice())
            );
        }
        assert_eq!(got.links(), entry.links());
    }
    assert_eq!(
        tree.root().children().count(),
        expected.root().children().count()
    );
}

fn read_sample<R: std::io::Read>(input: R, op: &str, expected: &Tree, verify: bool) -> u128 {
    let mut reader = CpioReader::new(StdIo::new(input));
    let mut buf = vec![0; 65536];
    let start = Instant::now();
    if op == "tree" {
        let tree = read_tree(&mut reader).unwrap();
        let elapsed = start.elapsed().as_nanos();
        if verify {
            check_tree(&tree, expected);
        }
        black_box(&tree);
        elapsed
    } else {
        let mut entries = 0;
        while let Some(mut entry) = reader.next_entry().unwrap() {
            black_box(entry.path());
            entries += 1;
            if op == "read" {
                loop {
                    let n = entry.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    black_box(&buf[..n]);
                }
            }
        }
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(entries, expected.root().children().count());
        elapsed
    }
}

fn write_sample<W: std::io::Write>(out: W, tree: &Tree, options: &CpioOptions) -> u128 {
    let start = Instant::now();
    black_box(write(StdIo::new(out), tree, options).unwrap());
    start.elapsed().as_nanos()
}

fn sample(
    tree: &Tree,
    image: &[u8],
    options: &CpioOptions,
    op: &str,
    buffered: bool,
    verify: bool,
) -> (u128, Counts) {
    let counts = Rc::new(Cell::new(Counts::default()));
    let elapsed = if op == "write" {
        let mut output = Counted {
            inner: Vec::with_capacity(image.len()),
            counts: counts.clone(),
        };
        let elapsed = if buffered {
            write_sample(BufWriter::with_capacity(8192, &mut output), tree, options)
        } else {
            write_sample(&mut output, tree, options)
        };
        if verify {
            assert_eq!(output.inner, image);
        }
        elapsed
    } else {
        let input = Counted {
            inner: std::io::Cursor::new(image),
            counts: counts.clone(),
        };
        if buffered {
            read_sample(BufReader::with_capacity(8192, input), op, tree, verify)
        } else {
            read_sample(input, op, tree, verify)
        }
    };
    (elapsed, counts.get())
}

fn main() {
    let mut samples = 7;
    let mut smoke = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--samples" => samples = args.next().unwrap().parse::<usize>().unwrap(),
            "--smoke" => {
                samples = 1;
                smoke = true;
            }
            "--csv" | "--bench" => {}
            _ => panic!("unknown argument: {arg}"),
        }
    }
    assert!(samples > 0);
    println!(
        "format,workload,operation,buffer_bytes,median_ns,reads,read_bytes,writes,write_bytes,flushes,max_read,max_write"
    );
    for format in [Format::Newc, Format::Crc, Format::Odc] {
        let options = CpioOptions::new().with_format(format);
        for workload in [
            "small",
            "large",
            "links",
            "metadata",
            "stream-small",
            "stream-large",
        ] {
            let (tree, _source) = fixture(workload);
            let mut output = StdIo::new(Vec::new());
            write(&mut output, &tree, &options).unwrap();
            let image = output.into_inner();
            for op in ["read", "skip", "tree", "write"] {
                for buffered in [false, true] {
                    let (_, expected) = sample(&tree, &image, &options, op, buffered, true);
                    let mut timings = Vec::new();
                    for _ in 0..samples {
                        let (elapsed, counts) =
                            sample(&tree, &image, &options, op, buffered, smoke);
                        assert_eq!(counts, expected);
                        timings.push(elapsed);
                    }
                    timings.sort_unstable();
                    println!(
                        "{format:?},{workload},{op},{},{},{},{},{},{},{},{},{}",
                        if buffered { 8192 } else { 0 },
                        timings[samples / 2],
                        expected.reads,
                        expected.read_bytes,
                        expected.writes,
                        expected.write_bytes,
                        expected.flushes,
                        expected.max_read,
                        expected.max_write
                    );
                }
            }
        }
    }
}

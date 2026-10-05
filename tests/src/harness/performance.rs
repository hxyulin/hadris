use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hadris_io::{Error, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

/// Requested device I/O, including failed calls; bytes are requested bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoCounts {
    pub read_calls: u64,
    pub read_bytes: u64,
    pub write_calls: u64,
    pub write_bytes: u64,
    pub flush_calls: u64,
    pub failures: u64,
    pub seek_calls: u64,
}

/// Place below a cache to measure backend I/O, or above it for driver requests.
pub struct Counted<D> {
    inner: D,
    counts: Rc<Cell<IoCounts>>,
}

impl<D> Counted<D> {
    pub fn new(inner: D) -> (Self, Rc<Cell<IoCounts>>) {
        let counts = Rc::new(Cell::new(IoCounts::default()));
        (
            Self {
                inner,
                counts: counts.clone(),
            },
            counts,
        )
    }
}

impl<D: ErrorType> ErrorType for Counted<D> {
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
    fn disk_offset(&self) -> u64 {
        self.inner.disk_offset()
    }
    fn writable(&self) -> bool {
        self.inner.writable()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        let result = self.inner.read_blocks(first, buf);
        let mut counts = self.counts.get();
        counts.read_calls += 1;
        counts.read_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        let result = self.inner.write_blocks(first, buf);
        let mut counts = self.counts.get();
        counts.write_calls += 1;
        counts.write_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }

    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        let result = self.inner.flush();
        let mut counts = self.counts.get();
        counts.flush_calls += 1;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }
}

/// Counts requested stream reads/writes for peers using `std::io`.
pub struct CountedStream<D> {
    inner: D,
    counts: Rc<Cell<IoCounts>>,
}

impl<D> CountedStream<D> {
    pub fn new(inner: D) -> (Self, Rc<Cell<IoCounts>>) {
        let counts = Rc::new(Cell::new(IoCounts::default()));
        (
            Self {
                inner,
                counts: counts.clone(),
            },
            counts,
        )
    }
}

impl<D: std::io::Read> std::io::Read for CountedStream<D> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let result = self.inner.read(buf);
        let mut counts = self.counts.get();
        counts.read_calls += 1;
        counts.read_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }
}

impl<D: std::io::Write> std::io::Write for CountedStream<D> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let result = self.inner.write(buf);
        let mut counts = self.counts.get();
        counts.write_calls += 1;
        counts.write_bytes += buf.len() as u64;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let result = self.inner.flush();
        let mut counts = self.counts.get();
        counts.flush_calls += 1;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }
}

impl<D: std::io::Seek> std::io::Seek for CountedStream<D> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let result = self.inner.seek(pos);
        let mut counts = self.counts.get();
        counts.seek_calls += 1;
        counts.failures += u64::from(result.is_err());
        self.counts.set(counts);
        result
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Measurement {
    pub elapsed: Duration,
    pub io: IoCounts,
}

impl Measurement {
    /// Setup, verification and destruction of the returned value are excluded.
    pub fn run<T>(counts: &Cell<IoCounts>, job: impl FnOnce() -> T) -> (T, Self) {
        counts.set(IoCounts::default());
        let start = Instant::now();
        let result = job();
        let elapsed = start.elapsed();
        (
            result,
            Self {
                elapsed,
                io: counts.get(),
            },
        )
    }

    pub const CSV_HEADER: &str = "format,workload,sample,elapsed_ns,read_calls,read_bytes,write_calls,write_bytes,flush_calls,io_failures,seek_calls";

    pub fn csv_row(&self, format: &str, workload: &str, sample: usize) -> String {
        let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
        let io = self.io;
        format!(
            "{},{},{sample},{},{},{},{},{},{},{},{}",
            quote(format),
            quote(workload),
            self.elapsed.as_nanos(),
            io.read_calls,
            io.read_bytes,
            io.write_calls,
            io.write_bytes,
            io.flush_calls,
            io.failures,
            io.seek_calls
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hadris_storage::MemDevice;

    #[test]
    fn stream_errors_preserve_the_error_and_count_failures() {
        use std::io::{self, Read, Seek, SeekFrom, Write};
        struct Refused;
        impl Read for Refused {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        impl Write for Refused {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        impl Seek for Refused {
            fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        let (mut stream, counts) = CountedStream::new(Refused);
        let (_, measured) = Measurement::run(&counts, || {
            assert_eq!(
                stream.read(&mut [0; 4]).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                stream.write(&[0; 8]).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                stream.flush().unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                stream.seek(SeekFrom::Start(0)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        });
        assert_eq!(
            measured.io,
            IoCounts {
                read_calls: 1,
                read_bytes: 4,
                write_calls: 1,
                write_bytes: 8,
                flush_calls: 1,
                seek_calls: 1,
                failures: 4
            }
        );
    }

    #[test]
    fn stream_counts_requests_and_forwards_seeks() {
        use std::io::{Cursor, Read, Seek, SeekFrom, Write};
        let (mut stream, counts) = CountedStream::new(Cursor::new(vec![1, 2]));
        let (_, first) = Measurement::run(&counts, || {
            let mut bytes = [0; 4];
            assert_eq!(stream.read(&mut bytes).unwrap(), 2);
            assert_eq!(bytes, [1, 2, 0, 0]);
            assert_eq!(stream.seek(SeekFrom::Start(0)).unwrap(), 0);
            stream.write_all(&[3, 4, 5, 6]).unwrap();
            stream.flush().unwrap();
        });
        assert_eq!(
            first.io,
            IoCounts {
                read_calls: 1,
                read_bytes: 4,
                write_calls: 1,
                write_bytes: 4,
                flush_calls: 1,
                failures: 0,
                seek_calls: 1
            }
        );
        let (_, second) = Measurement::run(&counts, || {
            stream.seek(SeekFrom::Start(0)).unwrap();
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, [3, 4, 5, 6]);
        });
        assert_eq!(
            second.io,
            IoCounts {
                read_calls: 1,
                read_bytes: 4,
                seek_calls: 1,
                ..IoCounts::default()
            }
        );
    }

    #[test]
    fn counter_placement_distinguishes_cache_hits() {
        let sector = BlockSize::new(512).unwrap();
        let (dev, backend) = Counted::new(MemDevice::new(vec![7; 1024], sector));
        let cache = hadris_storage::sync::Cache::new(dev, 2);
        let (mut dev, requests) = Counted::new(cache);
        let (_, measured) = Measurement::run(&requests, || {
            for _ in 0..2 {
                let mut buf = [0; 512];
                dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
                assert_eq!(buf, [7; 512]);
            }
        });
        assert_eq!(measured.io.read_calls, 2);
        assert_eq!(measured.io.read_bytes, 1024);
        assert_eq!(backend.get().read_calls, 1);
        assert_eq!(backend.get().read_bytes, 512);
    }

    #[test]
    fn preserves_read_only_devices_and_failed_writes() {
        let bytes = [0; 512];
        let (mut dev, counts) =
            Counted::new(MemDevice::new(&bytes[..], BlockSize::new(512).unwrap()));
        assert!(!dev.writable());
        let (result, measured) =
            Measurement::run(&counts, || dev.write_blocks(BlockIndex::new(0), &[1; 512]));
        assert_eq!(result.unwrap_err().kind(), hadris_io::ErrorKind::ReadOnly);
        assert_eq!(measured.io.write_calls, 1);
        assert_eq!(measured.io.write_bytes, 512);
        assert_eq!(measured.io.failures, 1);
        assert_eq!(bytes, [0; 512]);
    }

    #[test]
    fn counts_failures_and_resets_between_measurements() {
        let (mut dev, counts) =
            Counted::new(MemDevice::new(vec![0; 1024], BlockSize::new(512).unwrap()));
        assert!(dev.writable());
        assert_eq!(dev.block_count(), 2);
        assert_eq!(dev.max_block_count(), 2);
        assert_eq!(dev.disk_offset(), 0);
        let (_, first) = Measurement::run(&counts, || {
            dev.read_blocks(BlockIndex::new(0), &mut [0; 512]).unwrap();
            dev.write_blocks(BlockIndex::new(1), &[1; 512]).unwrap();
            assert!(dev.read_blocks(BlockIndex::new(2), &mut [0; 512]).is_err());
            dev.flush().unwrap();
        });
        assert_eq!(
            first.io,
            IoCounts {
                read_calls: 2,
                read_bytes: 1024,
                write_calls: 1,
                write_bytes: 512,
                flush_calls: 1,
                failures: 1,
                seek_calls: 0
            }
        );
        let (_, second) = Measurement::run(&counts, || dev.flush().unwrap());
        assert_eq!(
            second.io,
            IoCounts {
                flush_calls: 1,
                ..IoCounts::default()
            }
        );
        assert!(
            second
                .csv_row("a,b", "say \"hi\"", 2)
                .starts_with("\"a,b\",\"say \"\"hi\"\"\",2,")
        );
    }
}

//! The same adapter scenarios through the sync and the async adapters give
//! the same data, error kinds, messages and locations.
#![cfg(all(feature = "sync", feature = "async", feature = "alloc"))]

use hadris_io::{Error, ErrorKind, ErrorType, Location};
use hadris_storage::{BlockSize, MemDevice};

type Outcome = Result<Vec<u8>, (ErrorKind, &'static str, Option<Location>)>;

fn outcome<T, E>(result: Result<T, Error<E>>, data: impl FnOnce(T) -> Vec<u8>) -> Outcome {
    result
        .map(data)
        .map_err(|e| (e.kind(), e.message(), e.location()))
}

const B4: BlockSize = match BlockSize::new(4) {
    Some(size) => size,
    None => panic!(),
};

fn counting(len: u8) -> Vec<u8> {
    (0..len).collect()
}

fn mem(len: u8) -> MemDevice<Vec<u8>> {
    MemDevice::new(counting(len), B4)
}

/// A device that reports 4 blocks of 4 bytes but holds 16 more behind them
/// and leaves bounds checks to its callers.
struct Lax(Vec<u8>);

impl ErrorType for Lax {
    type Error = core::convert::Infallible;
}

impl Lax {
    fn new() -> Self {
        Self(counting(32))
    }

    fn read(&self, first: u64, buf: &mut [u8]) {
        let at = first as usize * 4;
        buf.copy_from_slice(&self.0[at..at + buf.len()]);
    }

    fn write(&mut self, first: u64, buf: &[u8]) {
        let at = first as usize * 4;
        self.0[at..at + buf.len()].copy_from_slice(buf);
    }
}

impl hadris_storage::sync::BlockDevice for Lax {
    fn block_size(&self) -> BlockSize {
        B4
    }
    fn block_count(&self) -> u64 {
        4
    }
    fn writable(&self) -> bool {
        true
    }
    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        self.read(first.get(), buf);
        Ok(())
    }
    fn write_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        self.write(first.get(), buf);
        Ok(())
    }
}

impl hadris_storage::async_::BlockDevice for Lax {
    type State = ();
    fn block_size(&self) -> BlockSize {
        B4
    }
    fn block_count(&self) -> u64 {
        4
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut core::task::Context<'_>,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
        self.read(first.get(), buf);
        core::task::Poll::Ready(Ok(()))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut core::task::Context<'_>,
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
        self.write(first.get(), buf);
        core::task::Poll::Ready(Ok(()))
    }
    fn cancel(&mut self, _: &mut ()) {}
}

macro_rules! scenarios {
    ($go:ident) => {{
        let mut out: Vec<(&'static str, Outcome)> = Vec::new();
        let mut buf = [0u8; 8];

        let mut dev = mem(18);
        let r = $go!(dev.read_blocks(BlockIndex::new(1), &mut buf));
        out.push(("mem read", outcome(r, |()| buf.to_vec())));
        let r = $go!(dev.read_blocks(BlockIndex::new(3), &mut buf));
        out.push(("mem read past end", outcome(r, |()| buf.to_vec())));
        let r = $go!(dev.read_blocks(BlockIndex::new(0), &mut buf[..3]));
        out.push(("mem read unaligned", outcome(r, |()| buf.to_vec())));
        let r = $go!(dev.read_blocks(BlockIndex::new(u64::MAX), &mut buf));
        out.push(("mem read overflow", outcome(r, |()| buf.to_vec())));
        let r = $go!(dev.write_blocks(BlockIndex::new(4), &[1; 4]));
        out.push(("mem write past end", outcome(r, |()| Vec::new())));
        let bytes = counting(8);
        let mut rom = MemDevice::new(&bytes[..], B4);
        let r = $go!(rom.write_blocks(BlockIndex::new(0), &[1; 4]));
        out.push(("mem write read-only", outcome(r, |()| Vec::new())));
        let mut ro = ReadOnly::new(mem(8));
        let r = $go!(ro.write_blocks(BlockIndex::new(0), &[1; 4]));
        out.push(("read-only adapter write", outcome(r, |()| Vec::new())));

        let mut image: Vec<u8> = Vec::new();
        let r = $go!(image.write_blocks(BlockIndex::new(1), &[5; 512]));
        out.push(("vec grow", outcome(r, |()| image.clone())));
        let mut big = [0u8; 1024];
        let r = $go!(image.read_blocks(BlockIndex::new(1), &mut big));
        out.push(("vec read past end", outcome(r, |()| big.to_vec())));
        let r = $go!(image.write_blocks(BlockIndex::new(u64::MAX / 512), &[0; 512]));
        out.push(("vec write past max", outcome(r, |()| Vec::new())));

        let mut part = Partition::new(mem(32), 8, 16);
        let r = $go!(part.read_blocks(BlockIndex::new(1), &mut buf[..4]));
        out.push(("partition read", outcome(r, |()| buf[..4].to_vec())));
        let r = $go!(part.write_blocks(BlockIndex::new(3), &[9; 4]));
        out.push((
            "partition write",
            outcome(r, |()| part.get_ref().get_ref().clone()),
        ));
        let r = $go!(part.read_blocks(BlockIndex::new(3), &mut buf));
        out.push(("partition read past end", outcome(r, |()| buf.to_vec())));
        let mut skew = Partition::new(mem(32), 2, 8);
        let r = $go!(skew.read_blocks(BlockIndex::new(0), &mut buf[..4]));
        out.push(("unaligned partition", outcome(r, |()| buf.to_vec())));
        let mut short = Partition::new(mem(16), 8, 16);
        let r = $go!(short.read_blocks(BlockIndex::new(2), &mut buf[..4]));
        out.push(("partition past device end", outcome(r, |()| buf.to_vec())));
        let mut lax = Partition::new(Lax::new(), 8, 16);
        let r = $go!(lax.read_blocks(BlockIndex::new(2), &mut buf[..4]));
        out.push(("partition past lax device", outcome(r, |()| buf.to_vec())));
        let r = $go!(lax.write_blocks(BlockIndex::new(2), &[7; 4]));
        out.push((
            "partition write past lax device",
            outcome(r, |()| lax.get_ref().0.clone()),
        ));
        let mut grow = Partition::new(Vec::<u8>::new(), 512, 4096);
        let r = $go!(grow.write_blocks(BlockIndex::new(2), &[3; 512]));
        out.push((
            "partition grows a vec",
            outcome(r, |()| grow.get_ref().clone()),
        ));

        let mut view = ByteView::new(mem(32));
        let mut bytes = [0u8; 6];
        let r = $go!(view.read_at(3, &mut bytes));
        out.push(("view partial read", outcome(r, |()| bytes.to_vec())));
        let r = $go!(view.write_at(5, &[0xAA; 7]));
        out.push((
            "view partial write",
            outcome(r, |()| view.get_ref().get_ref().clone()),
        ));
        let r = $go!(view.write_at(8, &[0xBB; 8]));
        out.push((
            "view whole write",
            outcome(r, |()| view.get_ref().get_ref().clone()),
        ));
        let r = $go!(view.read_at(30, &mut bytes));
        out.push(("view read past end", outcome(r, |()| bytes.to_vec())));
        let r = $go!(view.write_at(u64::MAX, &[1]));
        out.push(("view write overflow", outcome(r, |()| Vec::new())));
        let r = $go!(view.seek(SeekFrom::End(-2)));
        out.push(("view seek", outcome(r, |p: u64| p.to_le_bytes().to_vec())));
        let r = $go!(view.read(&mut bytes));
        out.push(("view read tail", outcome(r, |n| bytes[..n].to_vec())));
        let r = $go!(view.seek(SeekFrom::Current(-40)));
        out.push((
            "view seek negative",
            outcome(r, |p: u64| p.to_le_bytes().to_vec()),
        ));
        let r = $go!(view.seek(SeekFrom::Start(1)));
        out.push((
            "view seek start",
            outcome(r, |p: u64| p.to_le_bytes().to_vec()),
        ));
        let r = $go!(view.write(&[0xCC; 3]));
        out.push((
            "view stream write",
            outcome(r, |n| {
                let mut data = view.get_ref().get_ref().clone();
                data.push(n as u8);
                data
            }),
        ));

        let mut cache = Cache::new(mem(32), 2);
        let r = $go!(cache.write_blocks(BlockIndex::new(1), &[1; 4]));
        out.push((
            "cache first write",
            outcome(r, |()| cache.get_ref().get_ref().clone()),
        ));
        let r = $go!(cache.write_blocks(BlockIndex::new(2), &[2; 4]));
        out.push((
            "cache buffered write",
            outcome(r, |()| {
                let mut data = cache.get_ref().get_ref().clone();
                data.push(cache.is_dirty() as u8);
                data
            }),
        ));
        let mut wide = [0u8; 16];
        let r = $go!(cache.read_blocks(BlockIndex::new(0), &mut wide));
        out.push(("cache wide read", outcome(r, |()| wide.to_vec())));
        let r = $go!(cache.read_blocks(BlockIndex::new(2), &mut buf[..4]));
        out.push(("cache hit", outcome(r, |()| buf[..4].to_vec())));
        let r = $go!(cache.read_blocks(BlockIndex::new(7), &mut buf));
        out.push(("cache read past end", outcome(r, |()| buf.to_vec())));
        let r = $go!(cache.write_blocks(BlockIndex::new(0), &[0; 32]));
        out.push((
            "cache pass-through write",
            outcome(r, |()| cache.get_ref().get_ref().clone()),
        ));
        let r = $go!(cache.write_blocks(BlockIndex::new(5), &[5; 4]));
        out.push(("cache write", outcome(r, |()| Vec::new())));
        let r = $go!(cache.flush());
        out.push((
            "cache flush",
            outcome(r, |()| {
                let mut data = cache.get_ref().get_ref().clone();
                data.push(cache.is_dirty() as u8);
                data
            }),
        ));

        let mut ahead = ReadAhead::new(mem(32), 4);
        let r = $go!(ahead.read_blocks(BlockIndex::new(0), &mut buf[..4]));
        out.push(("read-ahead miss", outcome(r, |()| buf[..4].to_vec())));
        let r = $go!(ahead.read_blocks(BlockIndex::new(1), &mut buf[..4]));
        out.push(("read-ahead adjacent", outcome(r, |()| buf[..4].to_vec())));
        let r = $go!(ahead.write_blocks(BlockIndex::new(2), &[9; 4]));
        out.push(("read-ahead write", outcome(r, |()| Vec::new())));
        let r = $go!(ahead.read_blocks(BlockIndex::new(2), &mut buf));
        out.push(("read-ahead after write", outcome(r, |()| buf.to_vec())));
        let r = $go!(ahead.read_blocks(BlockIndex::new(7), &mut buf));
        out.push(("read-ahead past end", outcome(r, |()| buf.to_vec())));
        let mut wide = [0u8; 24];
        let r = $go!(ahead.read_blocks(BlockIndex::new(1), &mut wide));
        out.push(("read-ahead bypass", outcome(r, |()| wide.to_vec())));
        out.push(("read-ahead capacity", Ok(vec![ahead.capacity() as u8])));

        out
    }};
}

mod sync_side {
    use super::*;
    use hadris_io::SeekFrom;
    use hadris_io::sync::{Read as _, Seek as _, Write as _};
    use hadris_storage::sync::{BlockDevice as _, ByteView, Cache, ReadAhead};
    use hadris_storage::{BlockIndex, Partition, ReadOnly};

    macro_rules! now {
        ($e:expr) => {
            $e
        };
    }

    pub fn run() -> Vec<(&'static str, Outcome)> {
        scenarios!(now)
    }
}

mod async_side {
    use super::*;
    use hadris_io::SeekFrom;
    use hadris_io::async_::{Read as _, Seek as _, Write as _};
    use hadris_storage::async_::{BlockDevice as _, ByteView, Cache, ReadAhead};
    use hadris_storage::{BlockIndex, Partition, ReadOnly};

    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        let mut future = core::pin::pin!(future);
        loop {
            if let core::task::Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    macro_rules! wait {
        ($e:expr) => {
            block_on($e)
        };
    }

    pub fn run() -> Vec<(&'static str, Outcome)> {
        scenarios!(wait)
    }
}

#[test]
fn sync_and_async_adapters_agree() {
    let sync = sync_side::run();
    let r#async = async_side::run();
    assert_eq!(sync.len(), r#async.len());
    for (s, a) in sync.iter().zip(&r#async) {
        assert_eq!(s, a, "scenario {:?}", s.0);
    }
}

#[test]
fn partitions_stop_at_the_device_end() {
    let expected: Outcome = Err((
        ErrorKind::InvalidInput,
        "block request past the end of the device",
        Some(Location::Block(4)),
    ));
    for side in [sync_side::run(), async_side::run()] {
        let find = |name: &str| side.iter().find(|(n, _)| *n == name).unwrap().1.clone();
        assert_eq!(find("partition past lax device"), expected);
        assert_eq!(find("partition past device end"), expected);
        assert_eq!(find("partition write past lax device"), expected);
        assert_eq!(
            find("unaligned partition"),
            Err((
                ErrorKind::InvalidInput,
                "partition is not aligned to the device blocks",
                None
            ))
        );
        assert_eq!(
            find("view read past end"),
            Err((
                ErrorKind::InvalidInput,
                "byte range past the end of the device",
                Some(Location::Byte(30))
            ))
        );
        assert_eq!(
            find("mem write read-only"),
            Err((ErrorKind::ReadOnly, "memory buffer is read-only", None))
        );
    }
}

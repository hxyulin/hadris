#![cfg(any(feature = "sync", feature = "async"))]

use core::fmt;

use hadris_io::{ErrorType, SeekFrom};
use hadris_storage::BlockSize;

#[derive(Debug, PartialEq, Eq)]
struct Fault;

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("injected seek failure")
    }
}

impl core::error::Error for Fault {}

struct Stream {
    marker: u32,
    position: u64,
    fail: bool,
}

impl Stream {
    fn new() -> Self {
        Self {
            marker: 42,
            position: 0,
            fail: true,
        }
    }

    fn seek_to_end(&mut self, pos: SeekFrom) -> Result<u64, Fault> {
        assert_eq!(pos, SeekFrom::End(0));
        self.position = 16;
        if self.fail {
            Err(Fault)
        } else {
            Ok(self.position)
        }
    }
}

impl ErrorType for Stream {
    type Error = Fault;
}

#[cfg(feature = "sync")]
impl hadris_io::sync::Seek for Stream {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Fault> {
        self.seek_to_end(pos)
    }
}

#[cfg(feature = "sync")]
#[test]
fn sync_constructors_return_the_stream_and_original_cause() {
    use hadris_storage::sync::StreamDevice;

    for growable in [false, true] {
        let size = BlockSize::new(4).unwrap();
        let result = if growable {
            StreamDevice::new_growable(Stream::new(), size)
        } else {
            StreamDevice::new(Stream::new(), size)
        };
        let failure = result.err().expect("the injected seek must fail");
        assert_eq!(failure.stream().marker, 42);
        assert_eq!(failure.error().device_error(), Some(&Fault));
        let source = core::error::Error::source(&failure).unwrap();
        assert!(source.is::<hadris_io::Error<Fault>>());
        assert_eq!(format!("{failure}"), "stream-device construction failed");
        let (error, mut stream) = failure.into_parts();
        assert_eq!(error.device_error(), Some(&Fault));
        assert_eq!(stream.position, 16);
        stream.fail = false;
        let device = StreamDevice::new(stream, size).unwrap();
        assert_eq!(device.get_ref().marker, 42);
    }
}

#[cfg(all(feature = "sync", feature = "std"))]
#[test]
fn constructor_errors_propagate_into_standard_io_with_the_cause() {
    fn construct() -> std::io::Result<()> {
        let _device =
            hadris_storage::sync::StreamDevice::new(Stream::new(), BlockSize::new(4).unwrap())?;
        Ok(())
    }

    let error = construct().unwrap_err();
    let context = error
        .get_ref()
        .unwrap()
        .downcast_ref::<hadris_io::Error<Fault>>()
        .unwrap();
    assert_eq!(context.device_error(), Some(&Fault));
}

#[cfg(feature = "async")]
mod asynchronous {
    use super::{Fault, Stream};
    use core::future::Future;
    use core::task::{Context, Poll, Waker};
    use hadris_io::{Error, Location, SeekFrom};
    use hadris_storage::async_::{BlockDevice, StreamDevice};
    use hadris_storage::{BlockIndex, BlockSize};

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = core::pin::pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test stream completes each operation immediately"),
        }
    }

    impl hadris_storage::async_::Stream for Stream {
        type State = ();

        fn poll_read(
            &mut self,
            _: &mut (),
            _: &mut Context<'_>,
            _: &mut [u8],
        ) -> Poll<Result<usize, Error<Fault>>> {
            Poll::Ready(Ok(0))
        }

        fn poll_seek(
            &mut self,
            _: &mut (),
            _: &mut Context<'_>,
            pos: SeekFrom,
        ) -> Poll<Result<u64, Error<Fault>>> {
            Poll::Ready(self.seek_to_end(pos).map_err(|error| {
                Error::device(error, "measuring failed")
                    .with_location(Location::Byte(self.position))
            }))
        }

        fn cancel(&mut self, _: &mut ()) {}
    }

    #[test]
    fn failed_construction_preserves_context_and_allows_retry() {
        let size = BlockSize::new(4).unwrap();
        let failure = ready(StreamDevice::new(Stream::new(), size))
            .err()
            .expect("the injected seek must fail");
        assert_eq!(failure.error().device_error(), Some(&Fault));
        assert_eq!(failure.error().location(), Some(Location::Byte(16)));
        let mut stream = failure.into_stream();
        assert_eq!(stream.marker, 42);
        assert_eq!(stream.position, 16);
        stream.fail = false;
        let device = ready(StreamDevice::new(stream, size)).unwrap();
        assert_eq!(device.block_count(), 4);
    }

    #[test]
    fn operation_debug_does_not_require_a_debug_device() {
        let mut device =
            StreamDevice::with_block_count(Stream::new(), BlockSize::new(4).unwrap(), 4);
        let mut buf = [0; 4];
        let operation = device.read_blocks(BlockIndex::new(0), &mut buf);
        let text = format!("{operation:?}");
        assert!(text.contains("read"));
        assert!(text.contains("done: false"));
    }
}

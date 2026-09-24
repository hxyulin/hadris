#![cfg(feature = "async")]

use hadris_io::Cursor;
use hadris_io::r#async::{ByteSource, Read, Seek};

fn assert_send<T: Send>(value: T) -> T {
    value
}

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

async fn read_header<R: Read + Seek>(mut reader: R) -> [u8; 2] {
    let mut buf = [0u8; 2];
    reader.seek(hadris_io::SeekFrom::Start(1)).await.unwrap();
    reader.read_exact(&mut buf).await.unwrap();
    buf
}

async fn read_source<S: ByteSource>(mut source: S) -> u8 {
    let mut buf = [0u8; 1];
    source.read_exact_at(2, &mut buf).await.unwrap();
    buf[0]
}

#[test]
fn generic_futures_are_send() {
    let data = [1u8, 2, 3, 4];
    let header = block_on(assert_send(read_header(Cursor::new(&data))));
    assert_eq!(header, [2, 3]);
    let byte = block_on(assert_send(read_source(&data[..])));
    assert_eq!(byte, 3);
}

//! Runs the unified async driver experiment without an executor dependency.

use std::cell::Cell;
use std::future::{Future, poll_fn};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use hadris_example_unified_async::{
    Borrowed, Device, Fs, SendDevice, SendFileSystem, generic_send_read, require_send,
};

struct Memory<'a>(&'a [u8]);
impl SendDevice for Memory<'_> {
    async fn read(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        let bytes = self.0.get(offset..).unwrap_or_default();
        let n = buf.len().min(bytes.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        n
    }
    async fn flush(&mut self) {}
}

struct LocalMemory<'a> {
    bytes: &'a [u8],
    polls: Rc<Cell<usize>>,
}
impl Device for LocalMemory<'_> {
    async fn read(&mut self, offset: usize, buf: &mut [u8]) -> usize {
        let state = Rc::clone(&self.polls);
        let mut pending = true;
        poll_fn(move |cx| {
            state.set(state.get() + 1);
            if pending {
                pending = false;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        let bytes = self.bytes.get(offset..).unwrap_or_default();
        let n = buf.len().min(bytes.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        n
    }
    async fn flush(&mut self) {}
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn borrowed_local() {
    let bytes = Vec::from(b"borrowed data".as_slice());
    let polls = Rc::new(Cell::new(0));
    let mut device = LocalMemory {
        bytes: &bytes,
        polls: Rc::clone(&polls),
    };
    let mut fs = Fs::new(Borrowed(&mut device));
    let mut out = [0; 13];
    assert_eq!(block_on(fs.read(0, &mut out)), 13);
    assert_eq!(&out, b"borrowed data");
    block_on(fs.sync());
    assert!(polls.get() >= 8);
}

fn borrowed_send() {
    let bytes = Vec::from(b"borrowed data".as_slice());
    let mut device = Memory(&bytes);
    let mut fs = Fs::new(&mut device);
    let mut out = [0; 13];
    require_send(fs.read(0, &mut out));
    require_send(fs.sync());
    require_send(generic_send_read(&mut fs, 0, &mut out));
    require_send(hadris_example_unified_async::peer::generic_read(
        &mut fs, 0, &mut out,
    ));
    assert_eq!(block_on(generic_send_read(&mut fs, 0, &mut out)), 13);
    assert_eq!(&out, b"borrowed data");
}

fn across_thread() {
    let bytes = Vec::from(b"thread data".as_slice());
    let mut fs = Fs::new(Memory(&bytes));
    let mut out = [0; 11];
    std::thread::scope(|scope| {
        let future = generic_send_read(&mut fs, 0, &mut out);
        assert_eq!(scope.spawn(move || block_on(future)).join().unwrap(), 11);
    });
    assert_eq!(&out, b"thread data");
}

fn main() {
    borrowed_local();
    borrowed_send();
    across_thread();
    println!(
        "One driver type: borrowed local, borrowed Send, generic Send and cross-thread execution passed."
    );
    println!(
        "Send driver state: {} bytes; local driver state: {} bytes.",
        std::mem::size_of::<Fs<Memory<'_>>>(),
        std::mem::size_of::<Fs<LocalMemory<'_>>>()
    );
    let mut fs = Fs::new(Memory(b"state"));
    let mut buf = [0; 5];
    println!(
        "Inherent read future: {} bytes.",
        std::mem::size_of_val(&fs.read(0, &mut buf))
    );
    println!(
        "Refined read future: {} bytes.",
        std::mem::size_of_val(&SendFileSystem::read(&mut fs, 0, &mut buf))
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_send_borrowed_device() {
        borrowed_local();
    }
    #[test]
    fn send_borrowed_device_and_generic_future() {
        borrowed_send();
    }
    #[test]
    fn send_operation_runs_on_another_thread() {
        across_thread();
    }
}

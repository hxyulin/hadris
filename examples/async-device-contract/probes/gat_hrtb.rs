#![allow(dead_code)]
use std::future::{Future, Ready, ready};
trait Device {
    type Read<'a>: Future<Output = ()>
    where
        Self: 'a;
    fn read(&mut self) -> Self::Read<'_>;
}
trait SendDevice: Send + for<'a> Device<Read<'a>: Send> {}
impl<D> SendDevice for D where D: Send + for<'a> Device<Read<'a>: Send> {}
struct Fs<D>(D);
impl<D: Device> Fs<D> {
    async fn read(&mut self) {
        self.0.read().await;
    }
}
fn require_send(_: impl Future + Send) {}
fn generic<D: SendDevice>(fs: &mut Fs<D>) {
    require_send(fs.read());
}
struct Borrowed<'a>(&'a mut [u8]);
impl Device for Borrowed<'_> {
    type Read<'a>
        = Ready<()>
    where
        Self: 'a;
    fn read(&mut self) -> Self::Read<'_> {
        ready(())
    }
}
fn main() {
    let mut bytes = [0];
    let mut fs = Fs(Borrowed(&mut bytes));
    generic(&mut fs);
}

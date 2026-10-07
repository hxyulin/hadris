#![allow(dead_code)]
use std::future::{Future, Ready, ready};
trait Device {
    type Read<'a>: Future<Output = ()>
    where
        Self: 'a;
    fn read(&mut self) -> Self::Read<'_>;
}
trait SendDevice<'a>: Send + Device<Read<'a>: Send> + 'a {}
impl<'a, D> SendDevice<'a> for D where D: Send + Device<Read<'a>: Send> + 'a {}
struct Fs<D>(D);
impl<D: Device> Fs<D> {
    fn read(&mut self) -> D::Read<'_> {
        self.0.read()
    }
}
fn require_send(_: impl Future + Send) {}
fn generic<'a, D: SendDevice<'a>>(fs: &'a mut Fs<D>) {
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

#![allow(dead_code)]

//! A memory device a test steers while a volume is mounted on it: it fails
//! one write, refuses writes, and logs every write and flush.

use std::cell::RefCell;
use std::rc::Rc;

use hadris_io::{Error, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// A write of this many bytes at this byte offset.
    Write(u64, usize),
    Flush,
}

#[derive(Debug, Default)]
pub struct Script {
    /// The next write that covers this byte offset fails with a device
    /// error, and is then forgotten.
    pub fail_at: Option<u64>,
    /// Every write fails with `ErrorKind::ReadOnly`.
    pub refuse: bool,
    /// The writes and flushes that reached the device.
    pub log: Vec<Op>,
}

pub struct Scripted {
    pub inner: MemDevice<Vec<u8>>,
    pub script: Rc<RefCell<Script>>,
}

impl std::fmt::Debug for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scripted")
    }
}

impl Scripted {
    pub fn new(inner: MemDevice<Vec<u8>>) -> (Self, Rc<RefCell<Script>>) {
        let script = Rc::new(RefCell::new(Script::default()));
        let dev = Self {
            inner,
            script: script.clone(),
        };
        (dev, script)
    }

    pub fn into_image(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl ErrorType for Scripted {
    type Error = std::io::Error;
}

impl BlockDevice for Scripted {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner
            .read_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        let mut script = self.script.borrow_mut();
        if script.refuse {
            return Err(Error::new(
                hadris_fs::ErrorKind::ReadOnly,
                "write protected",
            ));
        }
        let at = first.get() * self.inner.block_size().get() as u64;
        if let Some(fail) = script.fail_at
            && (at..at + buf.len() as u64).contains(&fail)
        {
            script.fail_at = None;
            return Err(Error::device(
                std::io::Error::other("injected fault"),
                "write failed",
            ));
        }
        script.log.push(Op::Write(at, buf.len()));
        self.inner
            .write_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }

    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.script.borrow_mut().log.push(Op::Flush);
        Ok(())
    }
}

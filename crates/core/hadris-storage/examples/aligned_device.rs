//! Allocation-free adaptation of a controller that needs aligned buffers
//! and accepts one 512-byte block per transfer. Run with
//! `cargo run -p hadris-storage --example aligned_device`.

use core::convert::Infallible;
use hadris_io::{Error, ErrorKind, ErrorType, Location};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

const BLOCK: usize = 512;

#[repr(align(64))]
struct AlignedBlock([u8; BLOCK]);

trait Controller: ErrorType {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, first: u64, block: &mut AlignedBlock) -> Result<(), Self::Error>;
    fn write_block(&mut self, first: u64, block: &AlignedBlock) -> Result<(), Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
}

struct Device<C> {
    controller: C,
    scratch: AlignedBlock,
}

impl<C: Controller> Device<C> {
    fn new(controller: C) -> Self {
        Self {
            controller,
            scratch: AlignedBlock([0; BLOCK]),
        }
    }

    fn check(&self, first: BlockIndex, len: usize) -> Result<(), Error<C::Error>> {
        if len % BLOCK != 0
            || first
                .get()
                .checked_add((len / BLOCK) as u64)
                .is_none_or(|end| end > self.controller.block_count())
        {
            return Err(Error::new(ErrorKind::InvalidInput, "invalid block request")
                .with_location(Location::Block(first.get())));
        }
        Ok(())
    }
}

impl<C: Controller> ErrorType for Device<C> {
    type Error = C::Error;
}

impl<C: Controller> BlockDevice for Device<C> {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(BLOCK as u32).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.controller.block_count()
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.check(first, buf.len())?;
        for (index, chunk) in buf.chunks_exact_mut(BLOCK).enumerate() {
            let block = first.get() + index as u64;
            self.controller
                .read_block(block, &mut self.scratch)
                .map_err(|err| {
                    Error::device(err, "controller read failed")
                        .with_location(Location::Block(block))
                })?;
            chunk.copy_from_slice(&self.scratch.0);
        }
        Ok(())
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        self.check(first, buf.len())?;
        for (index, chunk) in buf.chunks_exact(BLOCK).enumerate() {
            let block = first.get() + index as u64;
            self.scratch.0.copy_from_slice(chunk);
            self.controller
                .write_block(block, &self.scratch)
                .map_err(|err| {
                    Error::device(err, "controller write failed")
                        .with_location(Location::Block(block))
                })?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.controller
            .flush()
            .map_err(|err| Error::device(err, "controller flush failed"))
    }
}

struct MemoryController {
    bytes: [u8; 8 * BLOCK],
    reads: usize,
    writes: usize,
}

impl ErrorType for MemoryController {
    type Error = Infallible;
}

impl Controller for MemoryController {
    fn block_count(&self) -> u64 {
        8
    }

    fn read_block(&mut self, first: u64, block: &mut AlignedBlock) -> Result<(), Self::Error> {
        assert_eq!(block.0.as_ptr() as usize % 64, 0);
        let start = first as usize * BLOCK;
        block.0.copy_from_slice(&self.bytes[start..start + BLOCK]);
        self.reads += 1;
        Ok(())
    }

    fn write_block(&mut self, first: u64, block: &AlignedBlock) -> Result<(), Self::Error> {
        assert_eq!(block.0.as_ptr() as usize % 64, 0);
        let start = first as usize * BLOCK;
        self.bytes[start..start + BLOCK].copy_from_slice(&block.0);
        self.writes += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn main() {
    let controller = MemoryController {
        bytes: [0; 8 * BLOCK],
        reads: 0,
        writes: 0,
    };
    let mut device = Device::new(controller);
    let input = AlignedBlock([7; BLOCK]);
    let mut unaligned = [0; 3 * BLOCK + 64];
    let at = (0..64)
        .find(|&at| unaligned[at..].as_ptr() as usize % 64 != 0)
        .unwrap();
    let buf = &mut unaligned[at..at + 3 * BLOCK];
    for block in buf.chunks_exact_mut(BLOCK) {
        block.copy_from_slice(&input.0);
    }
    device.write_blocks(BlockIndex::new(2), buf).unwrap();
    buf.fill(0);
    device.read_blocks(BlockIndex::new(2), buf).unwrap();
    assert_eq!(buf, &[7; 3 * BLOCK]);
    assert_eq!((device.controller.reads, device.controller.writes), (3, 3));
    assert_eq!(
        device
            .read_blocks(BlockIndex::new(7), buf)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        device
            .write_blocks(BlockIndex::new(0), &buf[..1])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!((device.controller.reads, device.controller.writes), (3, 3));
    device.flush().unwrap();
}

#[test]
fn unaligned_multiblock_requests_are_adapted() {
    main();
}

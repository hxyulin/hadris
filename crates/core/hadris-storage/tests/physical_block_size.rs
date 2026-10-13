#![cfg(feature = "alloc")]

use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition, ReadOnly};

const LOGICAL: BlockSize = BlockSize::new(512).unwrap();
const PHYSICAL: BlockSize = BlockSize::new(4096).unwrap();

/// A 512e disk: 512-byte logical blocks on 4096-byte physical blocks.
struct Advanced(MemDevice<Vec<u8>>);

impl Advanced {
    fn new() -> Self {
        Self(MemDevice::new(vec![0; 64 * 512], LOGICAL))
    }
}

impl ErrorType for Advanced {
    type Error = core::convert::Infallible;
}

#[cfg(feature = "sync")]
mod sync {
    use super::*;
    use hadris_storage::sync::{BlockDevice, Cache, ReadAhead};

    impl BlockDevice for Advanced {
        fn block_size(&self) -> BlockSize {
            self.0.block_size()
        }

        fn physical_block_size(&self) -> BlockSize {
            PHYSICAL
        }

        fn block_count(&self) -> u64 {
            self.0.block_count()
        }

        fn writable(&self) -> bool {
            true
        }

        fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            self.0.read_blocks(first, buf)
        }

        fn write_blocks(
            &mut self,
            first: BlockIndex,
            buf: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            self.0.write_blocks(first, buf)
        }
    }

    #[test]
    fn defaults_to_the_logical_block_size() {
        let dev = MemDevice::new(vec![0u8; 4096], LOGICAL);
        assert_eq!(dev.physical_block_size(), LOGICAL);
        assert_eq!(Vec::<u8>::new().physical_block_size(), LOGICAL);
    }

    #[test]
    fn wrappers_forward_it() {
        let mut dev = Advanced::new();
        assert_eq!(BlockDevice::physical_block_size(&&mut dev), PHYSICAL);
        let boxed: Box<Advanced> = Box::new(Advanced::new());
        assert_eq!(boxed.physical_block_size(), PHYSICAL);
        assert_eq!(
            Cache::new(Advanced::new(), 4).physical_block_size(),
            PHYSICAL
        );
        assert_eq!(
            ReadAhead::new(Advanced::new(), 4).physical_block_size(),
            PHYSICAL
        );
        assert_eq!(
            ReadOnly::new(Advanced::new()).physical_block_size(),
            PHYSICAL
        );
        let partition = Partition::new(Advanced::new(), 4096, 8192);
        assert_eq!(partition.physical_block_size(), PHYSICAL);
        assert_eq!(partition.block_size(), LOGICAL);
        let stacked = Cache::new(ReadOnly::new(Partition::new(Advanced::new(), 0, 4096)), 2);
        assert_eq!(stacked.physical_block_size(), PHYSICAL);
    }

    #[cfg(feature = "std")]
    #[test]
    fn image_files_report_the_logical_size_unless_set() {
        use hadris_storage::host::FileDevice;

        let path = std::env::temp_dir().join(format!("hadris-physical-{}", std::process::id()));
        std::fs::write(&path, [0u8; 8192]).unwrap();
        let dev = FileDevice::open(&path).unwrap();
        assert_eq!(dev.physical_block_size(), LOGICAL);
        let dev = dev.with_physical_block_size(PHYSICAL).unwrap();
        assert_eq!(dev.physical_block_size(), PHYSICAL);
        assert_eq!(dev.block_size(), LOGICAL);
        let odd = FileDevice::open(&path)
            .unwrap()
            .with_physical_block_size(BlockSize::new(1000).unwrap())
            .unwrap_err();
        assert_eq!(odd.kind(), std::io::ErrorKind::InvalidInput);
        let dev = FileDevice::open_with_block_size(&path, BlockSize::new(2048).unwrap()).unwrap();
        assert_eq!(dev.physical_block_size().get(), 2048);
        std::fs::remove_file(&path).unwrap();
    }
}

#[cfg(feature = "async")]
mod asynch {
    use super::*;
    use core::task::{Context, Poll};
    use hadris_storage::async_::{BlockDevice, Cache, ReadAhead};

    impl BlockDevice for Advanced {
        type State = ();

        fn block_size(&self) -> BlockSize {
            BlockDevice::block_size(&self.0)
        }

        fn physical_block_size(&self) -> BlockSize {
            PHYSICAL
        }

        fn block_count(&self) -> u64 {
            BlockDevice::block_count(&self.0)
        }

        fn poll_read_blocks(
            &mut self,
            state: &mut (),
            cx: &mut Context<'_>,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Poll<Result<(), Error<Self::Error>>> {
            self.0.poll_read_blocks(state, cx, first, buf)
        }

        fn cancel(&mut self, _: &mut ()) {}
    }

    #[test]
    fn wrappers_forward_it() {
        let mut dev = Advanced::new();
        assert_eq!(BlockDevice::physical_block_size(&&mut dev), PHYSICAL);
        let boxed: Box<Advanced> = Box::new(Advanced::new());
        assert_eq!(BlockDevice::physical_block_size(&boxed), PHYSICAL);
        assert_eq!(
            Cache::new(Advanced::new(), 4).physical_block_size(),
            PHYSICAL
        );
        assert_eq!(
            ReadAhead::new(Advanced::new(), 4).physical_block_size(),
            PHYSICAL
        );
        assert_eq!(
            BlockDevice::physical_block_size(&ReadOnly::new(Advanced::new())),
            PHYSICAL
        );
        let partition = Partition::new(Advanced::new(), 4096, 8192);
        assert_eq!(BlockDevice::physical_block_size(&partition), PHYSICAL);
        let plain = MemDevice::new(vec![0u8; 4096], LOGICAL);
        assert_eq!(BlockDevice::physical_block_size(&plain), LOGICAL);
    }
}

#![cfg(all(
    feature = "std",
    feature = "sync",
    feature = "async",
    feature = "write"
))]

use core::future::Future;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_io::SeekFrom;
use hadris_io::r#async::Seek;
use hadris_part::r#async::partition_table;
use hadris_part::sync::scheme_io::PartitionTableWriteExt;
use hadris_part::{GptPartitionEntry, Guid, PartitionSchemeType, PartitionTable};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[test]
fn async_leaf_detects_and_opens_validated_gpt_non_destructively() {
    let mut scheme = PartitionTable::new_gpt(8192, 512);
    let PartitionTable::Gpt { gpt, .. } = &mut scheme else {
        unreachable!();
    };
    gpt.add_partition(GptPartitionEntry::new(
        Guid::EFI_SYSTEM,
        Guid::from_bytes([0x42; 16]),
        40,
        4135,
    ))
    .unwrap();

    let mut image = std::io::Cursor::new(vec![0_u8; 8192 * 512]);
    scheme.write_to(&mut image).unwrap();
    let bytes = image.into_inner();

    block_on(async {
        let mut source = hadris_io::Cursor::new(bytes.as_slice());
        source.seek(SeekFrom::Start(73)).await.unwrap();
        assert_eq!(
            partition_table::detect(&mut source).await.unwrap(),
            PartitionSchemeType::Gpt
        );
        assert_eq!(source.stream_position().await.unwrap(), 73);

        let opened = partition_table::open(&mut source, 512).await.unwrap();
        opened.validate().unwrap();
        assert_eq!(opened.scheme_type(), PartitionSchemeType::Gpt);
        assert_eq!(opened.partitions().len(), 1);
    });
}

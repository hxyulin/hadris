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

use hadris_fat::r#async::FatVolume;
use hadris_fat::sync::FatVolumeWriteExt;

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
fn async_leaf_open_traverse_and_read_multicluster_file() {
    use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};

    let mut image = vec![0_u8; 2 * 1024 * 1024];
    let options = FatFormatOptions::new(image.len() as u64).fat_type(FatTypeSelection::Fat12);
    let fs = FatVolumeFormatter::format(std::io::Cursor::new(&mut image[..]), options).unwrap();
    let root = fs.root_dir();
    let nested = fs.create_dir(&root, "NESTED").unwrap();
    let entry = fs.create_file(&nested, "PAYLOAD.BIN").unwrap();
    let payload: Vec<u8> = (0..1537).map(|index| (index % 251) as u8).collect();
    let mut writer = fs.write_file(&entry).unwrap();
    assert_eq!(writer.write(&payload).unwrap(), payload.len());
    writer.finish().unwrap();
    drop(fs);

    block_on(async {
        let volume = FatVolume::open(hadris_io::Cursor::new(image.as_slice()))
            .await
            .unwrap();
        let nested = volume.open_dir_path("./NESTED").await.unwrap();
        assert!(nested.find("PAYLOAD.BIN").await.unwrap().is_some());

        let mut reader = volume.open_file_path("NESTED//PAYLOAD.BIN").await.unwrap();
        assert_eq!(reader.read_to_vec().await.unwrap(), payload);
        assert!(volume.open_path("../PAYLOAD.BIN").await.is_err());
    });
}

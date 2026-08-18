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
use hadris_io::SeekFrom;

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

        let mut reader = volume
            .open_file_path("NESTED//PAYLOAD.BIN")
            .await
            .unwrap()
            .with_buffer()
            .with_cached_chain()
            .await
            .unwrap();
        let mut chunk = [0_u8; 41];
        assert_eq!(reader.seek(SeekFrom::Start(700)).await.unwrap(), 700);
        assert_eq!(reader.read(&mut chunk).await.unwrap(), chunk.len());
        assert_eq!(&chunk, &payload[700..741]);
        assert_eq!(reader.seek(SeekFrom::Current(-50)).await.unwrap(), 691);
        assert_eq!(reader.read(&mut chunk[..9]).await.unwrap(), 9);
        assert_eq!(&chunk[..9], &payload[691..700]);
        assert_eq!(reader.seek(SeekFrom::End(-7)).await.unwrap(), 1530);
        assert_eq!(reader.read(&mut chunk).await.unwrap(), 7);
        assert_eq!(&chunk[..7], &payload[1530..]);
        assert_eq!(reader.seek(SeekFrom::Start(0)).await.unwrap(), 0);
        assert_eq!(reader.read_to_vec().await.unwrap(), payload);

        let mut late_cached = volume.open_file_path("NESTED/PAYLOAD.BIN").await.unwrap();
        late_cached.seek(SeekFrom::Start(700)).await.unwrap();
        let mut late_cached = late_cached.with_cached_chain().await.unwrap();
        late_cached.seek(SeekFrom::Start(0)).await.unwrap();
        assert_eq!(late_cached.read(&mut chunk[..1]).await.unwrap(), 1);
        assert_eq!(chunk[0], payload[0]);
        assert!(volume.open_path("../PAYLOAD.BIN").await.is_err());
    });
}

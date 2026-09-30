macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}
use hadris_fs::r#async::FileSystem;
#[allow(clippy::duplicate_mod)]
#[path = "mem_fs.rs"]
mod mem_fs;
pub use mem_fs::MemFs;

pub fn fixture() -> MemFs {
    super::fixture(MemFs::new, MemFs::add)
}

/// Returns `Pending` once, as a device waiting on an interrupt does.
pub async fn yield_now() {
    let mut yielded = false;
    core::future::poll_fn(|_| {
        if yielded {
            core::task::Poll::Ready(())
        } else {
            yielded = true;
            core::task::Poll::Pending
        }
    })
    .await
}

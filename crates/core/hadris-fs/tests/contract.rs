//! The in-memory test driver passes the contract kit in every mode.

#![cfg(all(feature = "contract", feature = "std"))]

mod common;

#[cfg(feature = "sync")]
#[test]
fn sync_mode() {
    use common::sync::MemFs;
    use hadris_fs::sync::{Volume, contract};

    let mut fs = MemFs::new();
    contract::check(&mut fs).unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
    let vol = Volume::new(MemFs::new());
    contract::check(&mut *vol.lock()).unwrap();
    contract::check(&mut Box::new(MemFs::new())).unwrap();
}

#[cfg(feature = "async")]
#[test]
fn async_mode() {
    use common::block_on;

    block_on(async {
        let mut fs = common::asynch::MemFs::new();
        hadris_fs::r#async::contract::check(&mut fs).await.unwrap();
        assert_eq!(fs.open_nodes(), 1);
        let vol = hadris_fs::r#async::Volume::new(common::asynch::MemFs::new());
        hadris_fs::r#async::contract::check(&mut *vol.lock().await)
            .await
            .unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut common::asynch::fixture().read_only())
            .await
            .unwrap();
    });
}

#[cfg(feature = "sync")]
#[test]
fn violations_name_the_broken_rule() {
    use common::sync::MemFs;
    use hadris_fs::sync::contract;

    let err = contract::check(&mut MemFs::new().read_only()).unwrap_err();
    assert_eq!(err.case(), "root");
    assert_eq!(
        err.to_string(),
        "root: the filesystem is writable (it did not)"
    );
}

#[cfg(feature = "sync")]
#[test]
fn read_only_drivers_pass_the_read_only_kit() {
    use common::sync::{MemFs, fixture};
    use hadris_fs::sync::contract;

    contract::check_read_only(&mut fixture().read_only()).unwrap();
    let err = contract::check_read_only(&mut MemFs::new()).unwrap_err();
    assert_eq!(err.rule(), "the filesystem is read-only");
}

//! The in-memory test driver passes the contract kit in every mode and
//! tier.

#![cfg(all(feature = "contract", feature = "std"))]

mod common;

#[cfg(feature = "sync")]
#[test]
fn sync_raw_and_shared_tiers() {
    use common::sync::MemFs;
    use hadris_fs::sync::{Volume, contract};

    contract::check(&mut MemFs::new()).unwrap();
    let vol = Volume::new(MemFs::new());
    contract::check(&mut &vol).unwrap();
    assert_eq!(vol.into_inner().open_nodes(), 1);
}

#[cfg(feature = "async")]
#[test]
fn async_modes() {
    use common::block_on;

    block_on(async {
        let mut fs = common::asynch::MemFs::new();
        hadris_fs::r#async::contract::check(&mut fs).await.unwrap();
        assert_eq!(fs.open_nodes(), 1);
        let vol = hadris_fs::r#async::Volume::new(common::asynch::MemFs::new());
        hadris_fs::r#async::contract::check(&mut &vol)
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

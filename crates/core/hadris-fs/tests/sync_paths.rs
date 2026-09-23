//! Path policies are types (S14): Lexical and Posix side by side, symlinks
//! followed without allocation, and pins released on every error.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use common::sync::fixture;
use hadris_fs::sync::{DriverExt, FsDriver, Lexical, PathExt, Posix, Resolver, Volume};
use hadris_fs::{ErrorKind, NodeId, OpenOptions};

#[test]
fn lexical_and_posix_side_by_side() {
    let lexical = Volume::local(fixture());
    let posix = Volume::local(fixture().with_resolver(Posix::new()));

    assert_eq!(lexical.read_to_vec("/missing/../a.txt").unwrap(), b"root a");
    let err = posix.read_to_vec("/missing/../a.txt").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);

    assert!(lexical.metadata("/a.txt/..").unwrap().file_type().is_dir());
    assert_eq!(
        posix.metadata("/a.txt/..").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        posix.metadata("/a.txt/").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );

    for path in [
        "/etc/up/conf",
        "/link/conf",
        "/abs",
        "/long",
        "/link/../link/conf",
    ] {
        assert_eq!(posix.read_to_vec(path).unwrap(), b"key=value", "{path}");
    }
    assert_eq!(
        lexical.read_to_vec("/link/conf").unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(lexical.lock().open_nodes(), 1);
    assert_eq!(posix.lock().open_nodes(), 1);
}

#[test]
fn posix_limits_and_errors() {
    let mut fs = fixture();
    let posix = Posix::new();
    assert_eq!(
        posix.resolve(&mut fs, "/loop").unwrap_err().kind(),
        ErrorKind::Symlink
    );
    assert_eq!(
        Posix::<16>.resolve(&mut fs, "/long").unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let dir = posix.resolve(&mut fs, "/link/").unwrap();
    assert!(fs.node_metadata(dir).unwrap().file_type().is_dir());
    fs.forget(dir);
    let node = Lexical.resolve(&mut fs, "/nope/../a.txt").unwrap();
    fs.forget(node);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn opening_an_unfollowed_symlink_fails() {
    let mut fs = fixture();
    let err = fs.open("/abs", OpenOptions::read()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Symlink);
    let mut fs = fs.with_resolver(Posix::new());
    assert_eq!(fs.read_to_vec("/abs").unwrap(), b"key=value");
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn raw_tier_picks_a_policy_per_call() {
    let mut fs = fixture();
    let node = Posix::new().resolve(&mut fs, "/etc/../a.txt").unwrap();
    assert_eq!(fs.node_metadata(node).unwrap().len(), 6);
    fs.forget(node);
    assert_ne!(FsDriver::root(&fs), NodeId::new(0));
}

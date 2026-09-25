//! `Walk` lists a tree depth first on a heap or a lent stack, stops at
//! loops and at its depth limit, and fuses after an error (R7).

#![cfg(feature = "std")]

mod common;

use common::MemError;
use hadris_fs::{ErrorKind, FileType, WalkEntry, WalkFrame};

fn tree<F>(new: impl FnOnce() -> F, add: impl Fn(&mut F, &str, &str, FileType, &[u8])) -> F {
    let mut fs = new();
    add(&mut fs, "/", "a", FileType::Dir, b"");
    add(&mut fs, "/a", "b", FileType::Dir, b"");
    add(&mut fs, "/a/b", "c.txt", FileType::File, b"c");
    add(&mut fs, "/a", "d.txt", FileType::File, b"d");
    add(&mut fs, "/", "e.txt", FileType::File, b"e");
    fs
}

fn label(entry: &WalkEntry) -> String {
    format!(
        "{}:{}",
        entry.depth(),
        entry.entry().name().to_str().unwrap()
    )
}

const ALL: [&str; 5] = ["1:a", "2:b", "3:c.txt", "2:d.txt", "1:e.txt"];

#[cfg(feature = "sync")]
mod sync {
    use super::*;
    use common::sync::MemFs;
    use hadris_fs::sync::{FileSystem, Walk};

    fn fixture() -> MemFs {
        tree(MemFs::new, MemFs::add)
    }

    fn collect(walk: &mut Walk<'_>, fs: &mut MemFs) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(entry) = walk.next(fs).unwrap() {
            out.push(label(&entry));
        }
        out
    }

    #[test]
    fn lists_depth_first() {
        let mut fs = fixture();
        let root = fs.root();
        assert_eq!(collect(&mut Walk::new(root), &mut fs), ALL);
        let mut stack = [WalkFrame::EMPTY; 3];
        assert_eq!(
            collect(&mut Walk::with_stack(root, &mut stack), &mut fs),
            ALL
        );
        assert_eq!(fs.open_nodes(), 1);
    }

    #[test]
    fn skip_dir_stays_out() {
        let mut fs = fixture();
        let mut walk = Walk::new(fs.root());
        let mut out = Vec::new();
        while let Some(entry) = walk.next(&mut fs).unwrap() {
            if entry.entry().name().as_bytes() == b"b" {
                walk.skip_dir();
            }
            out.push(label(&entry));
        }
        assert_eq!(out, ["1:a", "2:b", "2:d.txt", "1:e.txt"]);
    }

    #[test]
    fn a_short_stack_fails_and_fuses() {
        let mut fs = fixture();
        let mut stack = [WalkFrame::EMPTY; 2];
        let mut walk = Walk::with_stack(fs.root(), &mut stack);
        assert_eq!(label(&walk.next(&mut fs).unwrap().unwrap()), "1:a");
        assert_eq!(label(&walk.next(&mut fs).unwrap().unwrap()), "2:b");
        let err = walk.next(&mut fs).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
        assert!(walk.next(&mut fs).unwrap().is_none());

        let mut walk = Walk::with_stack(fs.root(), &mut []);
        let err = walk.next(&mut fs).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
        assert!(walk.next(&mut fs).unwrap().is_none());
    }

    #[test]
    fn a_loop_is_corrupt() {
        let mut fs = fixture();
        fs.alias("/a/b", "up", "/a");
        let mut walk = Walk::new(fs.root());
        let err = loop {
            match walk.next(&mut fs) {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the walk ended"),
                Err(err) => break err,
            }
        };
        assert_eq!(err.kind(), ErrorKind::Corrupt);
        assert!(walk.next(&mut fs).unwrap().is_none());
    }

    #[test]
    fn a_device_error_fuses() {
        let mut fs = fixture();
        let mut walk = Walk::new(fs.root());
        walk.next(&mut fs).unwrap().unwrap();
        fs.fail_next(MemError::Timeout { lba: 7 });
        let err = walk.next(&mut fs).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Io);
        assert!(walk.next(&mut fs).unwrap().is_none());
        assert!(walk.next(&mut fs).unwrap().is_none());
    }
}

#[cfg(feature = "async")]
mod asynch {
    use super::*;
    use common::asynch::MemFs;
    use common::block_on;
    use hadris_fs::r#async::{FileSystem, Walk};

    #[test]
    fn lists_depth_first_and_fuses() {
        block_on(async {
            let mut fs = tree(MemFs::new, MemFs::add);
            let root = fs.root();
            let mut out = Vec::new();
            let mut walk = Walk::new(root);
            while let Some(entry) = walk.next(&mut fs).await.unwrap() {
                out.push(label(&entry));
            }
            assert_eq!(out, ALL);

            let mut stack = [WalkFrame::EMPTY; 3];
            let mut walk = Walk::with_stack(root, &mut stack);
            walk.next(&mut fs).await.unwrap().unwrap();
            fs.fail_next(MemError::Timeout { lba: 7 });
            let err = walk.next(&mut fs).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Io);
            assert!(walk.next(&mut fs).await.unwrap().is_none());
        });
    }

    /// Generic over the filesystem, with no `Send` bounds: the mode's
    /// supertraits prove them.
    fn count<F: FileSystem + 'static>(mut fs: F) -> std::thread::JoinHandle<usize> {
        std::thread::spawn(move || {
            block_on(async move {
                let mut walk = Walk::new(fs.root());
                let mut n = 0;
                while walk.next(&mut fs).await.unwrap().is_some() {
                    n += 1;
                }
                n
            })
        })
    }

    #[test]
    fn walks_move_between_threads() {
        assert_eq!(count(tree(MemFs::new, MemFs::add)).join().unwrap(), 5);
    }
}

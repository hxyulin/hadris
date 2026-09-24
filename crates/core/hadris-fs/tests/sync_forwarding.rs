//! `&mut F` and `Box<F>` forward every trait method (R10). The methods are
//! read from the trait definition itself, so a method added to
//! `FileSystem` fails here until the forwarding macro has it too.

use std::collections::BTreeSet;

fn methods(source: &str) -> BTreeSet<&str> {
    source
        .split("fn ")
        .skip(1)
        .filter_map(|rest| rest.split(['(', '<']).next())
        .filter(|name| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .collect()
}

#[test]
fn forwarding_covers_the_trait() {
    let source = include_str!("../src/api/filesystem.rs");
    let start = source.find("pub trait FileSystem {").unwrap();
    let body = &source[start..];
    let declared = methods(&body[..body.find("\n}\n").unwrap()]);
    let start = source.find("macro_rules! forward_fs_methods").unwrap();
    let body = &source[start..];
    let forwarded = methods(&body[..body.find("\n}\n").unwrap()]);
    assert!(declared.len() > 20, "{declared:?}");
    assert_eq!(declared, forwarded);
}

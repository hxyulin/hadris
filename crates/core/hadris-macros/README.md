# hadris-macros

Proc macros that power dual sync/async APIs across the Hadris workspace.

Filesystem and partition crates write I/O code **once** with `async fn` / `.await`,
then compile it twice:

- under a `sync` module via [`strip_async!`](https://docs.rs/hadris-macros) (async keywords removed)
- under an `async` module unchanged

## `strip_async!`

Transforms a token tree for synchronous compilation:

- strips `async` before `fn`, `move`, and `unsafe`
- strips `.await`
- recurses into `{ }`, `( )`, and `[ ]` groups

```rust
hadris_macros::strip_async! {
    pub async fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<()> {
        reader.read_exact(buf).await
    }
}
// Becomes a synchronous `fn read_exact` with no `.await`.
```

## Consumer boilerplate

Each dual-API crate defines thin modules like this (see `hadris-part` / `hadris-fat`):

```rust,ignore
#[cfg(feature = "sync")]
pub mod sync {
    pub use hadris_io::sync::{Read, Write, Seek /* ... */};

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }
    macro_rules! sync_only {
        ($($item:tt)*) => { $($item)* };
    }
    macro_rules! async_only {
        ($($item:tt)*) => {};
    }

    #[path = "."]
    mod __inner {
        pub mod mbr_io; // shared source file
    }
    pub use __inner::*;
}

#[cfg(feature = "async")]
pub mod r#async {
    pub use hadris_io::r#async::{Read, Write, Seek /* ... */};

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* }; // keep async
    }
    macro_rules! sync_only {
        ($($item:tt)*) => {};
    }
    macro_rules! async_only {
        ($($item:tt)*) => { $($item)* };
    }

    #[path = "."]
    mod __inner {
        pub mod mbr_io;
    }
    pub use __inner::*;
}
```

Shared implementation files wrap bodies in `io_transform! { ... }` and use
`async fn` + `.await` throughout.

## Rules of thumb

1. **Write once** — put I/O logic in files included from both `sync` and `async` via `#[path]`.
2. **Doc comments inside macros** — rustdoc only sees docs that appear *inside* `io_transform!` / `strip_async!` invocations.
3. **`sync_only!` / `async_only!`** — use for code that cannot be shared (e.g. sync-only caches).
4. **Feature gates** — gate the modules with `sync` / `async` features; re-export `sync::*` at the crate root when `sync` is the default desktop path.

## License

Licensed under the [MIT license](../../LICENSE-MIT).

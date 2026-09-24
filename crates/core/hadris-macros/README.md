# hadris-macros

Proc macros that power dual sync/async APIs across the Hadris workspace.

Filesystem and partition crates write I/O code **once** with `async fn` / `.await`,
then compile it once per mode:

- under a `sync` module via [`strip_async!`](https://docs.rs/hadris-macros) (async keywords removed)
- under an `r#async` module via `send_async!`, whose trait methods return
  `Send` futures
- in `hadris-io` and `hadris-storage`, also under a `local` module
  unchanged, for executors whose futures are not `Send`

## `strip_async!`

Transforms a token tree for synchronous compilation:

- strips `async` before `fn`, `move`, and `unsafe`
- strips `.await`
- recurses into `{ }`, `( )`, and `[ ]` groups

```rust
use hadris_io::ExactError;
use hadris_io::sync::Read;

hadris_macros::strip_async! {
    pub async fn read_magic<R: Read>(reader: &mut R) -> Result<[u8; 4], ExactError<R::Error>> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic).await?;
        Ok(magic)
    }
}
// Becomes a synchronous `fn read_magic` with no `.await`.
```

## `send_async!`

Rewrites trait declarations so generic callers can prove futures `Send`:

- `async fn f(..) -> R` in a trait becomes `fn f(..) -> impl Future<Output = R> + Send`
- default bodies become `async move { .. }`
- the trait gains a `Send` supertrait, plus `Sync` when an async method takes `&self`
- impls and everything outside trait declarations pass through unchanged

## Consumer boilerplate

Each dual-API crate defines one module per mode that includes the same
source file and says how to transform it (see `hadris-part` and
`hadris-fat`):

```rust,ignore
#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async! { $($item)* } };
    }
    use hadris_storage::sync as storage;

    #[path = "io.rs"]
    mod io;
    pub use io::{open, read, scan};
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }
    use hadris_storage::r#async as storage;

    #[path = "io.rs"]
    mod io;
    pub use io::{open, read, scan};
}
```

The shared file wraps its items in `io_transform! { ... }`, writes
`async fn` and `.await` throughout, and names I/O traits through the
per-mode alias (`storage::BlockDevice`). A crate whose `r#async` code
defines no traits can pass the items through unchanged there, as
`hadris-part` does.

## Rules of thumb

1. **Write once.** Put I/O logic in files included from every mode module
   via `#[path]`.
2. **Doc comments inside macros.** Rustdoc only sees docs that appear inside
   the `io_transform!` invocation.
3. **Mode-only code.** Use per-mode `sync_only!` / `async_only!` macros, as
   `hadris-fs` does, for code that cannot be shared.
4. **No root re-exports.** Gate each mode module with its feature and name
   items through the module (`hadris_part::sync::read`); the crate root
   holds only mode-independent items.

## Documentation

- [Feature and capability guide](https://hxyulin.github.io/hadris/concepts/features)
- [API reference](https://docs.rs/hadris-macros)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).

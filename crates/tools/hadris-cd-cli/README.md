# hadris-cd

`hadris-cd` creates and verifies optical-disc images containing both ISO 9660
and UDF namespaces. The two filesystems share file payloads, providing legacy
ISO compatibility and modern UDF support in one image.

```console
hadris-cd create ./disc-root --output disc.iso --volume-name MY_DISC
hadris-cd info disc.iso
hadris-cd verify disc.iso
```

Bridge images default to ISO 9660:1999-style long filenames, Joliet level 3,
and UDF 1.02. Rock Ridge, El Torito BIOS/UEFI boot images, and hybrid MBR/GPT
layouts can be enabled through `create` options.

Use `hadris-iso` or `hadris-udf` when you need to browse or extract one
namespace independently.

## Installation

```bash
cargo install hadris-cd-cli
```

Or build the canonical `hadris-cd` binary from the workspace:

```bash
cargo build --release -p hadris-cd-cli
```

## Commands

| Command | Purpose |
|---|---|
| `create` | Build an ISO/UDF bridge image from a host directory |
| `info` | Inspect image layout and namespace metadata |
| `verify` | Open both namespaces and compare shared content |

Run `hadris-cd <command> --help` for format, boot, and output options.

## Validation

The built-in verifier checks the coordinated ISO and UDF views. Before shipping
boot or archival media, also validate with the independent tools used by the
target environment.

See the [image validation guide](https://hxyulin.github.io/hadris/guides/validate-images)
for suggested checks.

## Documentation

- [Create ISO filesystems](https://hxyulin.github.io/hadris/creation/iso)
- [Create UDF filesystems](https://hxyulin.github.io/hadris/creation/udf)
- [Library API](https://docs.rs/hadris-cd)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).

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

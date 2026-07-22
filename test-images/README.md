# Generated Hadris test images

These images were generated with Hadris 2.0.0-rc.3 from the fixtures in this
directory. Run commands from the repository root.

## Images

| Image | Purpose | Expected verification |
| --- | --- | --- |
| `output/standalone.iso` | ISO 9660 Level 2 with Joliet and Rock Ridge | Pass |
| `output/standalone.udf` | Standalone mastered UDF 2.01 | Pass |
| `output/standalone-fat32.img` | 64 MiB FAT32 image with long filenames | Pass |
| `output/bridge-simple.iso` | ISO/UDF 2.01 bridge with two root files | Pass; 2 shared entries |
| `output/bridge-nested.iso` | ISO/UDF 2.01 bridge with nested directories | Pass; 10 shared entries |
| `output/bridge-nested-rc3-bug.iso` | Original RC3 nested-bridge regression fixture | Fails bridge verification with `not a directory` |

The original broken nested bridge remains as a regression fixture. The fixed
`bridge-nested.iso` exercises sibling directories, deep nesting, empty files,
and shared ISO/UDF payload extents.

## Bridge commands

```bash
cargo run -q -p hadris-cd-cli --bin hadris-cd -- \
  info test-images/output/bridge-simple.iso

cargo run -q -p hadris-cd-cli --bin hadris-cd -- \
  verify test-images/output/bridge-simple.iso

cargo run -q -p hadris-cd-cli --bin hadris-cd -- \
  info test-images/output/bridge-nested.iso

cargo run -q -p hadris-cd-cli --bin hadris-cd -- \
  verify test-images/output/bridge-nested.iso
```

The successful verifier reports:

```text
Verified: test-images/output/bridge-simple.iso (2 shared entries)
```

Inspect the same bridge through each filesystem independently:

```bash
cargo run -q -p hadris-iso-cli --bin hadris-iso -- \
  info test-images/output/bridge-nested.iso
cargo run -q -p hadris-udf-cli --bin hadris-udf -- \
  info test-images/output/bridge-nested.iso
```

## Standalone ISO

```bash
cargo run -q -p hadris-iso-cli --bin hadris-iso -- \
  info test-images/output/standalone.iso
cargo run -q -p hadris-iso-cli --bin hadris-iso -- \
  tree test-images/output/standalone.iso
cargo run -q -p hadris-iso-cli --bin hadris-iso -- \
  cat test-images/output/standalone.iso README.TXT
cargo run -q -p hadris-iso-cli --bin hadris-iso -- \
  verify test-images/output/standalone.iso
```

`info` shows the primary descriptor, Joliet Level 3, Rock Ridge, volume size,
block size, and path-table size.

## Standalone UDF

```bash
cargo run -q -p hadris-udf-cli --bin hadris-udf -- \
  info test-images/output/standalone.udf
cargo run -q -p hadris-udf-cli --bin hadris-udf -- \
  tree test-images/output/standalone.udf
cargo run -q -p hadris-udf-cli --bin hadris-udf -- \
  cat test-images/output/standalone.udf README.txt
cargo run -q -p hadris-udf-cli --bin hadris-udf -- \
  verify test-images/output/standalone.udf
```

The UDF verifier checks the VRS, anchor, volume descriptor sequence, file set
descriptor, and root-directory readability.

## FAT32

```bash
cargo run -q -p hadris-fat-cli --bin hadris-fat -- \
  info test-images/output/standalone-fat32.img
cargo run -q -p hadris-fat-cli --bin hadris-fat -- \
  stat test-images/output/standalone-fat32.img
cargo run -q -p hadris-fat-cli --bin hadris-fat -- \
  tree test-images/output/standalone-fat32.img
cargo run -q -p hadris-fat-cli --bin hadris-fat -- \
  cat test-images/output/standalone-fat32.img README.txt
cargo run -q -p hadris-fat-cli --bin hadris-fat -- \
  verify test-images/output/standalone-fat32.img
```

The FAT verifier reports file, directory, and cluster counts and a final
pass/fail result.

## SHA-256

```text
ee8372b49d756ba3d133fcddbbdc37d823981296ae96059d1b229f42d837d821  bridge-nested-rc3-bug.iso
113511157ef6844d2a724ba2eb4673f4833f2ed89d5ad48bdc5d964d77924987  bridge-nested.iso
c13f969debffa5c28f3f1d9d27445c22f12bdc6298b9a10d3f02e59acd769301  bridge-simple.iso
00903061a1d4f160d6c43bd70a5c0ddb6d1b1d110b9f343bf74a24a590167e2b  standalone-fat32.img
ab449a3e1e5dadb9a1dc2bf93f75c159e6258dfc43a4590020216e80b757f8f5  standalone.iso
ca512b5c69aec119211648e8b97fd670a611f4d993084bb3727c0ae887ebd046  standalone.udf
```

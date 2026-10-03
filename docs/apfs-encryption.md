# APFS encryption and the Asahi read-only driver

The next driver milestone is password-unlocked, software-encrypted APFS volumes.
Native macOS tools provide the independent producer and reader oracle. This is
one part of the eventual Asahi FUSE goal; it does not establish access to an
Apple-silicon Mac's internal FileVault Data volume.

## Two encryption paths

| Target | Decryption path | What a disposable image can prove |
|---|---|---|
| Password-encrypted APFS on a disk image or external disk | Software APFS key handling and data decryption | Password validation, key selection, decrypted metadata and file reads |
| Internal Apple-silicon APFS Data volume | Secure Enclave key handling and NVMe inline encryption | APFS reader behavior only; the image cannot qualify the hardware unlock path |

Apple describes internal FileVault key handling as occurring in the Secure
Enclave, without exposing encryption keys directly to the CPU. Asahi documents
the Secure Enclave passing volume keys directly to the NVMe controller for
encryption and decryption in flight. A software AES implementation with the
user's password is therefore insufficient evidence for internal-volume access.
The Linux SEP/NVMe unlock path needs separate qualification.

- [Apple: Volume encryption with FileVault](https://support.apple.com/guide/security/volume-encryption-with-filevault-sec4c6dc1b6e/web)
- [Asahi: Apple Silicon Platform Security](https://asahilinux.org/docs/platform/security/)
- [Apple File System Reference](https://developer.apple.com/support/apple-file-system/Apple-File-System-Reference.pdf)

## Reproducible native fixtures

Run on macOS, from the repository root:

```sh
cargo build -p hadris-cli
python3 scripts/apfs-encryption-fixtures.py \
  --output /tmp/hadris-apfs-encryption-fixtures \
  --hadris target/debug/hadris
```

The output directory must not exist. The script creates two disposable 64 MiB
APFS images without partition maps. It attaches only the images it created,
checks that each container's physical store belongs to the attachment, and
detaches in cleanup. It does not accept a device or existing container as input.

The encrypted fixture is created with `diskutil apfs addVolume` and a public
test password supplied over standard input. The temporary initial volume is
removed only from the new image; this also accommodates images whose container
allows only one volume. No DMG wrapper encryption is used. Both volumes contain
identical deterministic files, a sparse file, a hard link and a symlink.

The script checks that the encrypted volume is locked, rejects the wrong
password, accepts the correct password, and locks again after image detachment
and reattachment. A native read-only remount must reproduce file hashes, link
identity and symlink targets. `diskutil verifyVolume` runs while that read-only
mount is available, so the native checker can access its crypto I/O context.

`manifest.json` records the public password, volume UUIDs, native file hashes,
macOS version and passed checks. Native verifier and crypto-user output are
retained alongside the images. Binary fixtures stay outside the repository.
On failure, partial artifacts remain for diagnosis; rerun with a new directory.

The optional `--hadris` check captures today's baseline: plaintext file reads
must match every native hash, and the encrypted volume must fail explicitly
with `encrypted B-tree nodes`. That baseline must change when actual unlock
support lands; it is not an encryption-success test.

## Use cases for the unlock API

| Scenario | Required outcome |
|---|---|
| Correct disk-user password | Mount, enumerate metadata and reproduce native file hashes |
| Incorrect or absent password | Refuse access before exposing decrypted filesystem state; retain the device on mount failure |
| Multiple crypto users or keys | Select the intended identity without silently choosing an unrelated key |
| Sparse, linked and fragmented files | Decrypt allocated extents using their crypto identifiers and preserve holes and inode identity |
| Damaged keybag, oversized KDF parameters or unsupported protection | Return a bounded, typed failure without uncontrolled allocation or KDF work |
| Sync, async and `no_std` with allocation | Share parsing and crypto logic; keep host prompting and keychain access outside the library |
| Internal storage on Asahi | Accept an appropriate storage/unlock backend; do not assume a portable software volume key exists |

Implement and test keybag parsing and password/key unwrapping before wiring
decryption into metadata and extent reads. Use vetted cryptographic primitives,
test vectors and the native images, and keep credentials out of diagnostics.
Do not treat an object checksum as cryptographic authentication. Decrypted
metadata must still pass the ordinary APFS structural and bounds checks.

Keep volume selection independent of credentials. Evolve a mount/unlock API
through these scenarios rather than adding APFS passwords to generic
`MountOptions`. The storage backend remains responsible for hardware inline
decryption; APFS software encryption remains format-specific.

After software encryption, the FUSE qualification work still includes xattrs
and compression used by ordinary macOS files, Unicode name comparison, scalable
metadata traversal, and real Asahi hardware unlock tests. Start with a read-only
FUSE mount of an explicitly selected Data volume; presenting macOS's combined
System/Data namespace is a separate operation.

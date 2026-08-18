---
title: Validate generated images
---

# Validate generated images

Round-trip tests through Hadris catch internal inconsistencies. Independent
tools catch different assumptions and are the best final check before shipping
an image.

## FAT

```bash
fsck.fat -vn disk.img
mdir -i disk.img ::
```

Use read-only or no-modify modes during validation. For partitioned disks, pass
the partition offset supported by the selected tool or extract the partition to
a temporary image first.

## ISO 9660

```bash
xorriso -indev image.iso -check_media
xorriso -indev image.iso -find / -print
```

For bootable media, also inspect the El Torito catalog and test the image in the
target firmware or virtual machine.

## UDF and bridge images

```bash
udfinfo image.udf
xorriso -indev bridge.iso -report_system_area plain
```

Mount tests are useful but should not be the only check: operating systems can
accept malformed images permissively or hide descriptor-level problems.

## CPIO

```bash
cpio -itv < archive.cpio
mkdir extracted
cd extracted
cpio -idmu < ../archive.cpio
```

Extract only trusted archives into a real directory. For untrusted input, list
entries and reject absolute paths, parent traversal, and unsafe special files
before extraction.

## A practical release check

1. Create the image with Hadris.
2. Reopen and inspect it with Hadris.
3. Validate it with at least one independent parser.
4. Test it in the actual consumer: firmware, kernel, OS, hypervisor, or device.
5. Preserve the failing image as a regression fixture when a compatibility
   issue is found.

External command names vary by operating system and distribution. Keep these
checks in project scripts or CI images where tool versions can be pinned.

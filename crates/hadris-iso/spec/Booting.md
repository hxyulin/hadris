# Specifications for Booting from an ISO-9660 filesystem

## Introduction

This document describes the specification for booting from an ISO-9660 filesystem.
There are different types of booting, split into two categories:

- [Booting from the El Torito boot record](#booting-from-cd)
- [Booting from a hard drive](#booting-from-hdd)

### Booting from CD
This is not commonly used anymore, but is still supported by some motherboards, and the majority of emulators.
Booting from a CD can be done by either the BIOS or the EFI firmware (UEFI).

Booting from BIOS requires a boot entry, with the Platform ID for BIOS, and a boot image.
There is a misconception that the boot image needs a MBR, but this is not the case. 
Partition tables are only used when booting from a hard drive.

Booting from UEFI requires a boot entry, with the Platform ID for UEFI, and a boot image.
There is a misconception that the boot image needs a GPT or MBR, but this is not the case.

### Booting from HDD

Booting from a hard drive is the most common way to boot, and is also the most flexible.
A hard drive includes:
- HDD
- SSD 
- USB

(Unfinished)

#!/usr/bin/env python3
"""Build and verify disposable software-encrypted APFS fixtures on macOS."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys


PASSWORD = b"hadris-public-fixture-password\n"


def run(*args, password=None, check=True):
    result = subprocess.run(args, input=password, capture_output=True, timeout=120)
    if check and result.returncode:
        raise RuntimeError(
            f"{args[0]} {args[1:]} failed:\n"
            + result.stdout.decode(errors="replace")
            + result.stderr.decode(errors="replace")
        )
    return result


def plist(*args):
    return plistlib.loads(run(*args).stdout)


def contents(root):
    result = {}
    for name in ["hello.txt", "nested/pattern.bin", "sparse.bin", "hardlink"]:
        data = (root / name).read_bytes()
        result[name] = {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    result["symlink"] = {"target": os.readlink(root / "symlink")}
    if (root / "hello.txt").stat().st_ino != (root / "hardlink").stat().st_ino:
        raise RuntimeError("native hard-link identity differs")
    return result


def populate(root):
    (root / "hello.txt").write_bytes(b"hadris encrypted APFS fixture\n")
    (root / "nested").mkdir()
    (root / "nested/pattern.bin").write_bytes(bytes(i % 251 for i in range(300_000)))
    with (root / "sparse.bin").open("wb") as file:
        file.write(b"begin\n")
        file.seek(1024 * 1024)
        file.write(b"end\n")
        file.truncate(2 * 1024 * 1024)
    os.link(root / "hello.txt", root / "hardlink")
    os.symlink("hello.txt", root / "symlink")


def volume(container):
    containers = plist("diskutil", "apfs", "list", "-plist", container)["Containers"]
    if len(containers) != 1 or len(containers[0]["Volumes"]) != 1:
        raise RuntimeError("expected one disposable container with one volume")
    return containers[0]["Volumes"][0]


def attached(image):
    entities = plist("hdiutil", "attach", "-plist", "-nomount", str(image))["system-entities"]
    device = entities[0]["dev-entry"]
    try:
        volumes = [e for e in entities if e.get("volume-kind") == "apfs"]
        if len(volumes) != 1:
            raise RuntimeError("expected one attached APFS volume")
        info = plist("diskutil", "info", "-plist", volumes[0]["dev-entry"])
        container = info["APFSContainerReference"]
        description = plist("diskutil", "apfs", "list", "-plist", container)["Containers"][0]
        stores = {"/dev/" + store["DeviceIdentifier"] for store in description["PhysicalStores"]}
        if stores != {device}:
            raise RuntimeError("APFS physical store does not match the newly attached image")
        return device, container
    except BaseException:
        run("hdiutil", "detach", "-quiet", device)
        raise


def build(directory, encrypted):
    name = "encrypted" if encrypted else "plain"
    image = directory / f"{name}.dmg"
    mount = directory / f"{name}-mount"
    mount.mkdir()
    run("hdiutil", "create", "-quiet", "-size", "64m", "-fs", "APFS",
        "-layout", "NONE", "-volname", "HadrisControl", str(image))
    device, container = attached(image)
    try:
        if encrypted:
            run("diskutil", "apfs", "deleteVolume", volume(container)["DeviceIdentifier"])
            run("diskutil", "apfs", "addVolume", container, "APFS", "HadrisEncrypted",
                "-stdinpassphrase", "-nomount", password=PASSWORD)
            item = volume(container)
            if not item["Encryption"] or not item["Locked"] or item["CryptoMigrationOn"]:
                raise RuntimeError("new APFS volume is not fully encrypted and locked")
            run("diskutil", "apfs", "unlockVolume", item["DeviceIdentifier"],
                "-stdinpassphrase", "-nomount", password=PASSWORD)
        item = volume(container)
        identifier = item["DeviceIdentifier"]
        run("diskutil", "mount", "-mountPoint", str(mount), identifier)
        populate(mount)
        expected = contents(mount)
        run("diskutil", "unmount", identifier)
    finally:
        run("hdiutil", "detach", "-quiet", device)

    device, container = attached(image)
    try:
        item = volume(container)
        identifier = item["DeviceIdentifier"]
        if encrypted:
            if not item["Locked"]:
                raise RuntimeError("encrypted volume did not remain locked after reattachment")
            wrong = run("diskutil", "apfs", "unlockVolume", identifier,
                        "-stdinpassphrase", "-nomount", password=b"wrong-password\n", check=False)
            if wrong.returncode == 0 or not volume(container)["Locked"]:
                raise RuntimeError("incorrect password unlocked the volume")
            run("diskutil", "apfs", "unlockVolume", identifier,
                "-stdinpassphrase", "-nomount", password=PASSWORD)
        run("diskutil", "mount", "readOnly", "-mountPoint", str(mount), identifier)
        if contents(mount) != expected:
            raise RuntimeError("native contents changed after reattachment")
        verification = run("diskutil", "verifyVolume", identifier).stdout.decode()
        (directory / f"{name}-verify.txt").write_text(verification)
        run("diskutil", "unmount", identifier)
        crypto = run("diskutil", "apfs", "listCryptoUsers", identifier).stdout.decode()
        (directory / f"{name}-crypto-users.txt").write_text(crypto)
        item = volume(container)
    finally:
        run("hdiutil", "detach", "-quiet", device)
        mount.rmdir()
    return {"image": image.name, "volume_uuid": item["APFSVolumeUUID"],
            "encrypted": item["Encryption"], "files": expected}


def check_driver(binary, directory, fixtures):
    for name, expected in fixtures[0]["files"].items():
        if "sha256" not in expected:
            continue
        data = run(str(binary), "apfs", "cat", str(directory / "plain.dmg"), "/" + name).stdout
        if len(data) != expected["size"] or hashlib.sha256(data).hexdigest() != expected["sha256"]:
            raise RuntimeError(f"Hadris plaintext read differs from native oracle: {name}")
    rejected = run(str(binary), "apfs", "ls", str(directory / "encrypted.dmg"), check=False)
    message = rejected.stderr.decode(errors="replace")
    if rejected.returncode == 0 or "encrypted B-tree nodes" not in message:
        raise RuntimeError("expected the current Hadris encrypted-volume rejection: " + message)
    (directory / "hadris-encrypted-rejection.txt").write_text(message)
    return {"plain_file_hashes_match": True, "encrypted_mount": "unsupported encrypted B-tree nodes"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path, help="new directory for retained fixtures")
    parser.add_argument("--hadris", type=Path, help="check a CLI binary against the current driver baseline")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("Apple's APFS tools require macOS")
    directory = args.output.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    fixtures = [build(directory, False), build(directory, True)]
    if fixtures[0]["files"] != fixtures[1]["files"]:
        raise RuntimeError("plain and encrypted fixture contents differ")
    report = {"encryption": "APFS software volume encryption; outer image is unencrypted",
              "public_test_password": PASSWORD.decode().strip(),
              "macos": run("sw_vers", "-productVersion").stdout.decode().strip(),
              "fixtures": fixtures,
              "checks": ["native read-only remount matches hashes and links",
                         "wrong password rejected", "correct password unlocks",
                         "encrypted volume relocks after image reattachment",
                         "diskutil verifyVolume passes"]}
    if args.hadris:
        report["driver_baseline"] = check_driver(args.hadris.resolve(), directory, fixtures)
    (directory / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Verified plaintext and encrypted APFS fixtures: {directory}")
    print("Public test password: " + PASSWORD.decode().strip())


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Plan, package and upload the unified CLI's tagged release binaries."""

import argparse
import gzip
import hashlib
import io
import json
import re
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = Path("crates/tools/hadris-cli/Cargo.toml")
TARGETS = {
    "x86_64-unknown-linux-musl": "ubuntu-24.04",
    "aarch64-unknown-linux-musl": "ubuntu-24.04-arm",
    "x86_64-apple-darwin": "macos-15-intel",
    "aarch64-apple-darwin": "macos-15",
    "x86_64-pc-windows-msvc": "windows-2025",
}


def run(*args: str, cwd: Path = ROOT) -> str:
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


def release_plan(tag: str, workspace_tag: str = "") -> dict:
    if not re.fullmatch(r"hadris-cli-v[0-9A-Za-z.-]+", tag):
        raise ValueError("expected an existing hadris-cli-v<version> tag")
    commit = run("git", "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
    manifest = tomllib.loads(run("git", "show", f"{commit}:{MANIFEST.as_posix()}"))
    package = manifest["package"]
    version = package["version"]
    if package["name"] != "hadris-cli" or tag != f"hadris-cli-v{version}":
        raise ValueError("CLI tag does not match the package version at its commit")
    if not any(binary["name"] == "hadris" for binary in manifest.get("bin", [])):
        raise ValueError("the tagged CLI must ship the unified hadris binary")
    if workspace_tag:
        if workspace_tag != f"v{version}":
            raise ValueError("workspace release must use the CLI's version")
        workspace_commit = run("git", "rev-parse", "--verify", f"refs/tags/{workspace_tag}^{{commit}}")
        if workspace_commit != commit:
            raise ValueError("workspace and CLI tags must name the same commit")
    return {"include": [
        {"target": target, "runner": runner, "commit": commit, "version": version}
        for target, runner in TARGETS.items()
    ]}


def archive_stem(version: str, target: str) -> str:
    if target not in TARGETS or not re.fullmatch(r"[0-9A-Za-z.-]+", version):
        raise ValueError("invalid CLI version or unsupported target")
    return f"hadris-cli-{version}-{target}"


def checksum(archive: Path) -> str:
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return f"{digest}  {archive.name}\n"


def package(source: Path, version: str, target: str, output: Path) -> Path:
    stem = archive_stem(version, target)
    manifest = tomllib.loads((source / MANIFEST).read_text())
    if manifest["package"]["version"] != version:
        raise ValueError("source manifest does not match the requested CLI version")
    windows = target.endswith("-windows-msvc")
    binary = source / "target" / target / "release" / ("hadris.exe" if windows else "hadris")
    files = [
        (binary.name, binary.read_bytes(), 0o755),
        ("README.md", (source / MANIFEST.parent / "README.md").read_bytes(), 0o644),
        ("LICENSE-MIT", (source / "LICENSE-MIT").read_bytes(), 0o644),
    ]
    epoch = int(run("git", "show", "-s", "--format=%ct", "HEAD", cwd=source))
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"{stem}.{'zip' if windows else 'tgz'}"
    if windows:
        stamp = datetime.fromtimestamp(max(epoch, 315532800), timezone.utc).timetuple()[:6]
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
            for name, data, mode in files:
                entry = zipfile.ZipInfo(f"{stem}/{name}", stamp)
                entry.create_system = 3
                entry.external_attr = (0o100000 | mode) << 16
                bundle.writestr(entry, data, compress_type=zipfile.ZIP_DEFLATED)
    else:
        with archive.open("wb") as stream:
            with gzip.GzipFile(fileobj=stream, mode="wb", filename="", mtime=epoch) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as bundle:
                    for name, data, mode in files:
                        entry = tarfile.TarInfo(f"{stem}/{name}")
                        entry.size = len(data)
                        entry.mode = mode
                        entry.mtime = epoch
                        bundle.addfile(entry, io.BytesIO(data))
    archive.with_suffix(archive.suffix + ".sha256").write_text(checksum(archive))
    return archive


def smoke_test(source: Path, version: str, target: str) -> None:
    binary = source / "target" / target / "release" / (
        "hadris.exe" if target.endswith("-windows-msvc") else "hadris"
    )
    if run(str(binary.resolve()), "--version") != f"hadris {version}":
        raise ValueError("built binary reports the wrong release version")
    run(str(binary.resolve()), "--help")
    for command in ("info", "ls", "stat", "cat", "extract"):
        run(str(binary.resolve()), "apfs", command, "--help")


def upload(tag: str, version: str, directory: Path) -> None:
    release = json.loads(run("gh", "release", "view", tag, "--json", "assets,isDraft"))
    if release["isDraft"]:
        raise ValueError("CLI assets require a published GitHub release")
    existing = {asset["name"] for asset in release["assets"]}
    archives = [
        directory / f"{archive_stem(version, target)}.{'zip' if target.endswith('-windows-msvc') else 'tgz'}"
        for target in TARGETS
    ]
    for archive in archives:
        digest = archive.with_suffix(archive.suffix + ".sha256")
        if not archive.is_file() or digest.read_text() != checksum(archive):
            raise ValueError(f"missing archive or invalid checksum: {archive.name}")
    for archive in archives:
        digest = archive.with_suffix(archive.suffix + ".sha256")
        if archive.name in existing and digest.name in existing:
            print(f"{tag}: {archive.name} already uploaded; skipping")
            continue
        with tempfile.TemporaryDirectory() as temporary:
            downloaded = Path(temporary)
            if archive.name in existing:
                run("gh", "release", "download", tag, "--pattern", archive.name, "--dir", temporary)
                digest = downloaded / digest.name
                digest.write_text(checksum(downloaded / archive.name))
            if digest.name in existing:
                run("gh", "release", "download", tag, "--pattern", digest.name, "--dir", temporary)
                if (downloaded / digest.name).read_text() != checksum(archive):
                    raise ValueError(f"existing checksum conflicts with {archive.name}")
            missing = [str(path) for path in (archive, digest) if path.name not in existing]
            run("gh", "release", "upload", tag, *missing)


def main() -> None:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    plan = commands.add_parser("plan")
    plan.add_argument("tag")
    plan.add_argument("--workspace-tag", default="")
    pack = commands.add_parser("package")
    pack.add_argument("--source", type=Path, required=True)
    pack.add_argument("--version", required=True)
    pack.add_argument("--target", choices=TARGETS, required=True)
    pack.add_argument("--output", type=Path, required=True)
    publish = commands.add_parser("upload")
    publish.add_argument("tag")
    publish.add_argument("--version", required=True)
    publish.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "plan":
        print(json.dumps(release_plan(args.tag, args.workspace_tag)))
    elif args.command == "package":
        smoke_test(args.source, args.version, args.target)
        print(package(args.source, args.version, args.target, args.output))
    else:
        upload(args.tag, args.version, args.directory)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Regression tests for tagged CLI binaries, binstall layout and upload retries."""

import contextlib
import importlib.util
import io
import json
import subprocess
import tempfile
import tomllib
import unittest
import zipfile
import tarfile
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("cli_release", Path(__file__).with_name("cli-release.py"))
CLI = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLI)
VERSION = "3.0.0-rc.1"
TAG = f"hadris-cli-v{VERSION}"


class CliReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / "source"
        self.source.mkdir()
        manifest = self.source / CLI.MANIFEST
        manifest.parent.mkdir(parents=True)
        manifest.write_text(
            f'[package]\nname = "hadris-cli"\nversion = "{VERSION}"\n'
            '[[bin]]\nname = "hadris"\npath = "src/main.rs"\n'
        )
        (manifest.parent / "README.md").write_text("Unified CLI, including APFS.\n")
        (self.source / "LICENSE-MIT").write_text("MIT license fixture.\n")
        for target in CLI.TARGETS:
            binary = self.source / "target" / target / "release" / (
                "hadris.exe" if "windows" in target else "hadris"
            )
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"fixture binary")
        self.git("init", "--quiet")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release-test@example.invalid")
        self.git("config", "commit.gpgSign", "false")
        self.git("config", "tag.gpgSign", "false")
        self.git("config", "core.hooksPath", "/dev/null")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "CLI release fixture")
        self.git("tag", "-a", TAG, "-m", "CLI RC1")
        self.git("tag", f"v{VERSION}")
        self.output = Path(self.temp.name) / "dist"
        self.remote = {}
        self.uploads = []

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.source, check=True, capture_output=True, text=True
        ).stdout.strip()

    def plan(self, tag=TAG, workspace_tag=""):
        with patch.object(CLI, "run", side_effect=lambda *args: self.git(*args[1:])):
            return CLI.release_plan(tag, workspace_tag)

    def package_all(self):
        return [CLI.package(self.source, VERSION, target, self.output) for target in CLI.TARGETS]

    def gh(self, *args):
        self.assertEqual(args[:2], ("gh", "release"))
        if args[2] == "view":
            return json.dumps({"isDraft": False, "assets": [{"name": name} for name in self.remote]})
        if args[2] == "download":
            name = args[args.index("--pattern") + 1]
            destination = Path(args[args.index("--dir") + 1]) / name
            destination.write_bytes(self.remote[name])
        elif args[2] == "upload":
            self.assertNotIn("--clobber", args)
            for path in args[4:]:
                asset = Path(path)
                self.assertNotIn(asset.name, self.remote)
                self.remote[asset.name] = asset.read_bytes()
                self.uploads.append(asset.name)
        else:
            self.fail(f"unexpected GitHub CLI call: {args}")
        return ""

    def upload(self):
        with patch.object(CLI, "run", side_effect=self.gh), contextlib.redirect_stdout(io.StringIO()):
            CLI.upload(TAG, VERSION, self.output)

    def test_plan_pins_every_native_target_to_the_release_commit(self):
        rows = self.plan(workspace_tag=f"v{VERSION}")["include"]
        self.assertEqual({row["target"] for row in rows}, set(CLI.TARGETS))
        self.assertTrue(all(row["commit"] == self.git("rev-parse", "HEAD") for row in rows))
        self.assertTrue(all(row["version"] == VERSION for row in rows))

    def test_plan_rejects_wrong_package_or_version(self):
        for tag in ("main", "hadris-apfs-cli-v3.0.0-rc.1", "hadris-cli-v3.0.0-rc.1;echo"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                self.plan(tag)
        self.git("tag", "hadris-cli-v3.0.0-rc.2")
        with self.assertRaisesRegex(ValueError, "package version"):
            self.plan("hadris-cli-v3.0.0-rc.2")

    def test_workspace_mirror_requires_same_version_and_commit(self):
        with self.assertRaisesRegex(ValueError, "CLI's version"):
            self.plan(workspace_tag="v3.0.0")
        self.git("commit", "--quiet", "--allow-empty", "-m", "Later commit")
        self.git("tag", "--force", f"v{VERSION}")
        with self.assertRaisesRegex(ValueError, "same commit"):
            self.plan(workspace_tag=f"v{VERSION}")

    def test_archive_layout_matches_binstall_metadata(self):
        manifest = tomllib.loads((CLI.ROOT / CLI.MANIFEST).read_text())
        metadata = manifest["package"]["metadata"]["binstall"]
        for target in CLI.TARGETS:
            archive = CLI.package(self.source, VERSION, target, self.output)
            windows = "windows" in target
            stem = CLI.archive_stem(VERSION, target)
            binary = f"{stem}/hadris{'.exe' if windows else ''}"
            if windows:
                with zipfile.ZipFile(archive) as bundle:
                    self.assertIn(binary, bundle.namelist())
                    self.assertEqual(bundle.read(binary), b"fixture binary")
            else:
                with tarfile.open(archive) as bundle:
                    self.assertEqual(bundle.getmember(binary).mode, 0o755)
                    self.assertEqual(bundle.extractfile(binary).read(), b"fixture binary")
                    self.assertIn(f"{stem}/LICENSE-MIT", bundle.getnames())
            values = {
                "name": "hadris-cli", "repo": "https://github.com/hxyulin/hadris",
                "version": VERSION, "target": target, "archive-suffix": archive.suffix,
                "bin": "hadris", "binary-ext": ".exe" if windows else "",
            }
            url = metadata["pkg-url"]
            path = metadata["bin-dir"]
            for key, value in values.items():
                url = url.replace("{ " + key + " }", value)
                path = path.replace("{ " + key + " }", value)
            self.assertEqual(url, f"{values['repo']}/releases/download/{TAG}/{archive.name}")
            self.assertEqual(path, binary)
            fmt = metadata.get("overrides", {}).get(target, {}).get("pkg-fmt", metadata["pkg-fmt"])
            self.assertEqual(fmt, "zip" if windows else "tgz")
            self.assertEqual(archive.with_suffix(archive.suffix + ".sha256").read_text(), CLI.checksum(archive))

    def test_archives_are_reproducible(self):
        for target in ("aarch64-apple-darwin", "x86_64-pc-windows-msvc"):
            archive = CLI.package(self.source, VERSION, target, self.output)
            original = archive.read_bytes()
            CLI.package(self.source, VERSION, target, self.output)
            self.assertEqual(archive.read_bytes(), original)

    def test_source_snapshot_packages_without_git_metadata(self):
        epoch = int(self.git("show", "-s", "--format=%ct", "HEAD"))
        with patch.object(CLI, "run", side_effect=AssertionError("snapshot must not need Git")):
            archive = CLI.package(self.source, VERSION, "aarch64-apple-darwin", self.output, epoch)
        self.assertTrue(archive.is_file())

    def test_smoke_test_requires_every_apfs_subcommand(self):
        with patch.object(CLI, "run", return_value=f"hadris {VERSION}") as run:
            CLI.smoke_test(self.source, VERSION, "aarch64-apple-darwin")
        self.assertEqual([call.args[1:] for call in run.call_args_list], [
            ("--version",), ("--help",),
            *[("apfs", command, "--help") for command in ("info", "ls", "stat", "cat", "extract")],
        ])

    def test_upload_is_complete_and_idempotent(self):
        self.package_all()
        self.upload()
        self.assertEqual(len(self.uploads), len(CLI.TARGETS) * 2)
        self.uploads.clear()
        self.upload()
        self.assertEqual(self.uploads, [])

    def test_upload_resumes_using_existing_archive_contents(self):
        archives = self.package_all()
        self.remote[archives[0].name] = b"original remote binary archive"
        self.upload()
        digest_name = archives[0].name + ".sha256"
        self.assertEqual(self.remote[archives[0].name], b"original remote binary archive")
        with tempfile.TemporaryDirectory() as temporary:
            preserved = Path(temporary) / archives[0].name
            preserved.write_bytes(self.remote[archives[0].name])
            self.assertEqual(self.remote[digest_name].decode(), CLI.checksum(preserved))

    def test_upload_resumes_when_only_checksum_exists(self):
        archives = self.package_all()
        self.remote[archives[0].name + ".sha256"] = CLI.checksum(archives[0]).encode()
        self.upload()
        self.assertIn(archives[0].name, self.uploads)
        self.assertNotIn(archives[0].name + ".sha256", self.uploads)

    def test_conflicting_existing_checksum_stops_upload(self):
        archives = self.package_all()
        self.remote[archives[0].name + ".sha256"] = b"conflicting checksum\n"
        with self.assertRaisesRegex(ValueError, "existing checksum conflicts"):
            self.upload()
        self.assertEqual(self.uploads, [])

    def test_missing_target_or_invalid_checksum_stops_upload(self):
        archives = self.package_all()
        archives[-1].unlink()
        with self.assertRaisesRegex(ValueError, "missing archive"):
            self.upload()
        self.assertEqual(self.uploads, [])
        self.package_all()
        archives[0].write_bytes(b"damaged archive")
        with self.assertRaisesRegex(ValueError, "invalid checksum"):
            self.upload()
        self.assertEqual(self.uploads, [])


if __name__ == "__main__":
    unittest.main()

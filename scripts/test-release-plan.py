#!/usr/bin/env python3
"""Regression tests for crate publication and workspace GitHub release plans."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("release-plan.py")
WORKFLOW = SCRIPT.parents[1] / ".github/workflows/release.yml"


class ReleasePlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copyfile(SCRIPT, self.root / "scripts/release-plan.py")
        self.packages = [
            {
                "name": "hadris",
                "version": "3.0.0-rc.1",
                "publish": None,
                "dependencies": [{"name": "hadris-fat-raw", "path": "raw", "kind": None}],
            },
            {"name": "hadris-fat-raw", "version": "0.1.0", "publish": None, "dependencies": []},
            {"name": "example", "version": "0.1.0", "publish": [], "dependencies": []},
        ]
        self.write_metadata()
        (self.root / "CHANGELOG.md").write_text(
            "## [joint] - 2026-10-06\n\nJoint release notes.\n\n"
            "## [hadris 3.0.0-rc.1] - 2026-10-06\n\nUmbrella release notes.\n"
        )
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\n"
            "import pathlib, sys\n"
            "assert sys.argv[1:] == ['metadata', '--no-deps', '--format-version', '1']\n"
            "print(pathlib.Path('metadata.json').read_text())\n"
        )
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"])
        self.git("init", "--quiet")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release-test@example.invalid")
        self.git("config", "core.hooksPath", "/dev/null")
        self.git("config", "commit.gpgSign", "false")
        self.git("config", "tag.gpgSign", "false")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Release fixture")

    def write_metadata(self):
        (self.root / "metadata.json").write_text(json.dumps({"packages": self.packages}))

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.root, check=True, capture_output=True, text=True
        ).stdout.strip()

    def plan(self, *args):
        return subprocess.run(
            [sys.executable, "scripts/release-plan.py", *args],
            cwd=self.root, env=self.env, capture_output=True, text=True,
        )

    def github_releases(self):
        lines = WORKFLOW.read_text().splitlines(keepends=True)
        start = lines.index("      - name: Tag and create releases\n")
        start = lines.index("        run: |\n", start) + 1
        end = start
        while end < len(lines) and (lines[end].startswith("          ") or not lines[end].strip()):
            end += 1
        script = textwrap.dedent("".join(lines[start:end]))
        self.assertTrue(script.strip())
        state = self.root / "github-releases.json"
        if not state.exists():
            state.write_text("[]")
            self.git("init", "--bare", "--quiet", "remote.git")
            self.git("remote", "add", "origin", str(self.root / "remote.git"))
            (self.root / "bin/python3").symlink_to(sys.executable)
            gh = self.root / "bin/gh"
            gh.write_text(
                f"#!{sys.executable}\n"
                "import json, pathlib, subprocess, sys\n"
                "path = pathlib.Path('github-releases.json')\n"
                "releases = json.loads(path.read_text())\n"
                "args = sys.argv[1:]\n"
                "assert args[:1] == ['release'], args\n"
                "if args[1] == 'view':\n"
                "    sys.exit(0 if any(r[2] == args[2] for r in releases) else 1)\n"
                "assert args[1] == 'create', args\n"
                "assert '--verify-tag' in args\n"
                "subprocess.run(['git', 'rev-parse', '--verify', 'refs/tags/' + args[2]], check=True)\n"
                "assert pathlib.Path(args[args.index('--notes-file') + 1]).read_text() == 'Joint release notes.\\n'\n"
                "releases.append(args)\n"
                "path.write_text(json.dumps(releases))\n"
            )
            gh.chmod(0o755)
        plan = self.plan("--github-plan", "--notes", "joint", "all")
        self.assertEqual(plan.returncode, 0, plan.stderr)
        result = subprocess.run(
            ["bash", "-c", script], cwd=self.root, capture_output=True, text=True,
            env=dict(self.env, CRATES="all", NOTES="joint", PLAN=plan.stdout.strip(),
                     GITHUB_SHA=self.git("rev-parse", "HEAD")),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(state.read_text())

    def test_joint_publication_contains_only_crates(self):
        result = self.plan("--notes", "joint", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), [
            "hadris-fat-raw 0.1.0 hadris-fat-raw-v0.1.0",
            "hadris 3.0.0-rc.1 hadris-v3.0.0-rc.1",
        ])

    def test_joint_github_release_uses_umbrella_version_and_shared_notes(self):
        result = self.plan("--github-plan", "--notes", "joint", "--notes-dir", "notes", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[-1], "Hadris 3.0.0-rc.1 v3.0.0-rc.1")
        for name in ("hadris", "hadris-fat-raw", "Hadris"):
            self.assertEqual((self.root / f"notes/{name}.md").read_text(), "Joint release notes.\n")

    def test_joint_stable_github_release(self):
        self.packages[0]["version"] = "3.0.0"
        self.write_metadata()
        result = self.plan("--github-plan", "--notes", "joint", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[-1], "Hadris 3.0.0 v3.0.0")

    def test_independent_umbrella_release_has_no_workspace_tag(self):
        result = self.plan("--github-plan", "hadris")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "hadris 3.0.0-rc.1 hadris-v3.0.0-rc.1\n")

    def test_joint_release_without_umbrella_has_no_workspace_tag(self):
        result = self.plan("--github-plan", "--notes", "joint", "hadris-fat-raw")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "hadris-fat-raw 0.1.0 hadris-fat-raw-v0.1.0\n")

    def test_existing_tags_on_same_commit_allow_rerun(self):
        self.git("tag", "-a", "v3.0.0-rc.1", "-m", "Workspace RC1")
        self.git("tag", "-a", "hadris-v3.0.0-rc.1", "-m", "Umbrella RC1")
        result = self.plan("--github-plan", "--notes", "joint", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Hadris 3.0.0-rc.1 v3.0.0-rc.1", result.stdout)

    def test_conflicting_workspace_tag_fails_before_publication(self):
        self.git("tag", "v3.0.0-rc.1")
        self.git("commit", "--quiet", "--allow-empty", "-m", "Later commit")
        for flag in ([], ["--github-plan"]):
            with self.subTest(flag=flag):
                result = self.plan(*flag, "--notes", "joint", "all")
                self.assertEqual(result.returncode, 1)
                self.assertIn("Hadris: tag v3.0.0-rc.1 already names", result.stderr)
                self.assertEqual(result.stdout, "")

    def test_conflicting_crate_tag_fails(self):
        self.git("tag", "hadris-v3.0.0-rc.1")
        self.git("commit", "--quiet", "--allow-empty", "-m", "Later commit")
        result = self.plan("--github-plan", "--notes", "joint", "all")
        self.assertEqual(result.returncode, 1)
        self.assertIn("hadris: tag hadris-v3.0.0-rc.1 already names", result.stderr)

    def test_missing_notes_fail_without_writing_files(self):
        result = self.plan("--github-plan", "--notes", "missing", "--notes-dir", "notes", "all")
        self.assertEqual(result.returncode, 1)
        self.assertIn("CHANGELOG.md needs exactly one", result.stderr)
        self.assertFalse((self.root / "notes").exists())

    def test_workflow_creates_rc_tags_and_prereleases_and_skips_reruns(self):
        releases = self.github_releases()
        self.assertEqual([r[2] for r in releases], [
            "hadris-fat-raw-v0.1.0", "hadris-v3.0.0-rc.1", "v3.0.0-rc.1",
        ])
        self.assertNotIn("--prerelease", releases[0])
        for release in releases:
            self.assertIn("--latest=false", release)
        for release in releases[1:]:
            self.assertIn("--prerelease", release)
        self.assertEqual(releases[-1][releases[-1].index("--title") + 1], "Hadris 3.0.0-rc.1")
        self.assertEqual(self.github_releases(), releases)

    def test_workflow_marks_only_stable_workspace_release_latest(self):
        self.packages[0]["version"] = "3.0.0"
        self.write_metadata()
        releases = self.github_releases()
        self.assertEqual([r[2] for r in releases if "--latest" in r], ["v3.0.0"])
        self.assertTrue(all("--prerelease" not in r for r in releases))


if __name__ == "__main__":
    unittest.main()

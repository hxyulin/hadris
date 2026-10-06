#!/usr/bin/env python3
"""Test semver baseline selection without compiling crate APIs."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


SCRIPT = Path(__file__).with_name("check-semver.sh").resolve()


def main():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory).resolve()

        def git(*args):
            return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

        git("init", "--quiet")
        git("config", "user.name", "Semver test")
        git("config", "user.email", "semver-test@example.invalid")
        git("config", "commit.gpgSign", "false")
        git("config", "tag.gpgSign", "false")
        manifest = root / "crates/hadris/Cargo.toml"
        manifest.parent.mkdir(parents=True)
        manifest.write_text('[package]\nname = "hadris"\nversion = "2.5.0"\n')
        git("add", ".")
        git("commit", "--quiet", "-m", "V2 baseline")
        baseline = git("rev-parse", "HEAD")
        manifest.write_text('[package]\nname = "hadris"\nversion = "3.0.0-rc.1"\n')
        new_manifest = root / "crates/hadris-fat-raw/Cargo.toml"
        new_manifest.parent.mkdir(parents=True)
        new_manifest.write_text('[package]\nname = "hadris-fat-raw"\nversion = "0.1.0"\n')
        git("add", ".")
        git("commit", "--quiet", "-m", "V3 with a new crate")

        packages = [
            {"name": "hadris", "version": "3.0.0-rc.1", "manifest_path": str(manifest)},
            {"name": "hadris-fat-raw", "version": "0.1.0", "manifest_path": str(new_manifest)},
        ]
        (root / "metadata.json").write_text(json.dumps({"packages": packages}))
        cargo = root / "bin/cargo"
        cargo.parent.mkdir()
        cargo.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
args = sys.argv[1:]
if args == ['metadata', '--no-deps', '--format-version', '1']:
    print(Path('metadata.json').read_text())
elif args[1:] == ['semver-checks', '--version']:
    print('cargo-semver-checks test')
elif args[1:3] == ['semver-checks', 'check-release']:
    Path('invocation.json').write_text(json.dumps(args[3:]))
    sys.exit(int(os.environ.get('SEMVER_TEST_EXIT', '0')))
else:
    sys.exit('Unexpected cargo arguments: ' + repr(args))
""")
        cargo.chmod(0o755)
        env = dict(os.environ, PATH=str(cargo.parent) + os.pathsep + os.environ["PATH"])

        def run(base, failure=0):
            (root / "invocation.json").unlink(missing_ok=True)
            result = subprocess.run(
                [shutil.which("bash"), str(SCRIPT), base], cwd=root,
                env=dict(env, SEMVER_TEST_EXIT=str(failure)), text=True, capture_output=True,
            )
            return result, root / "invocation.json"

        result, invocation = run(baseline)
        assert result.returncode == 0, result.stderr
        assert "hadris-fat-raw 0.1.0: not in" in result.stdout, result.stdout
        assert json.loads(invocation.read_text()) == ["--baseline-rev", baseline, "-p", "hadris"]

        result, _ = run(baseline, failure=1)
        assert result.returncode == 1, "A semver failure must fail the check"

        result, invocation = run("HEAD")
        assert result.returncode == 0 and not invocation.exists(), result.stderr
        git("tag", "hadris-v3.0.0-rc.1")
        result, invocation = run("HEAD")
        assert result.returncode == 0, result.stderr
        assert json.loads(invocation.read_text()) == [
            "--baseline-rev", git("rev-parse", "HEAD"), "--release-type", "minor", "-p", "hadris",
        ]
        result, invocation = run("missing-baseline")
        assert result.returncode != 0 and not invocation.exists()

    print("Semver baseline selection: new crates, release rules and failures passed")


if __name__ == "__main__":
    main()

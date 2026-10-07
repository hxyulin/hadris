#!/usr/bin/env python3

from __future__ import annotations

import re
import sys
import tempfile
import tomllib
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^]]*\]\(([^)\s]+)(?:\s+['\"][^)]*['\"])?\)")
EXTERNAL = ("http://", "https://", "mailto:")


def documentation_files() -> list[Path]:
    files = [
        ROOT / "README.md",
        ROOT / "examples" / "README.md",
        ROOT / "CONTRIBUTING.md",
        ROOT / "KNOWN_ISSUES.md",
    ]
    files.extend((ROOT / "docs").glob("**/*.md"))
    files.extend((ROOT / "crates").glob("**/README.md"))
    files.extend((ROOT / "website" / "docs").glob("**/*.md"))
    return sorted(files)


def resolve_link(source: Path, raw_target: str, root: Path = ROOT) -> Path | None:
    target = unquote(raw_target.strip("<>"))
    if target.startswith(EXTERNAL) or target.startswith("#"):
        return None
    target = target.split("#", 1)[0]
    if not target:
        return None
    if target.startswith("/img/"):
        return root / "website" / "static" / target.removeprefix("/")
    if target.startswith("/"):
        return None
    return (source.parent / target).resolve()


def check_links(files: list[Path], root: Path = ROOT) -> list[str]:
    root = root.resolve()
    errors = []
    for source in files:
        source = source.resolve()
        for line_number, line in enumerate(source.read_text().splitlines(), 1):
            for match in LINK.finditer(line):
                raw_target = match.group(1)
                target = resolve_link(source, raw_target, root)
                if target is None:
                    continue
                relative_source = source.relative_to(root)
                if not target.is_relative_to(root):
                    errors.append(
                        f"{relative_source}:{line_number}: link target escapes repository {raw_target}"
                    )
                elif not target.exists():
                    errors.append(
                        f"{relative_source}:{line_number}: missing link target {raw_target}"
                    )
    return errors


def check_toml(files: list[Path], root: Path = ROOT) -> list[str]:
    errors = []
    for source in files:
        contents = source.read_text()
        for match in re.finditer(r"```toml\n(.*?)```", contents, re.DOTALL):
            line_number = contents.count("\n", 0, match.start()) + 1
            try:
                tomllib.loads(match.group(1))
            except tomllib.TOMLDecodeError as error:
                errors.append(f"{source.relative_to(root)}:{line_number}: invalid TOML: {error}")
    return errors


def check_current_versions(files: list[Path], root: Path = ROOT) -> list[str]:
    versions = {}
    for manifest in (root / "crates").glob("**/Cargo.toml"):
        package = tomllib.loads(manifest.read_text())["package"]
        versions[package["name"]] = package["version"]
    errors = []
    for source in files:
        if not (
            source == root / "README.md"
            or source == root / "docs/hadris-3.0.0-migration.md"
            or source.is_relative_to(root / "website/docs")
            or (source.is_relative_to(root / "crates") and source.name == "README.md")
        ):
            continue
        contents = source.read_text()
        references = re.findall(r"https://docs\.rs/(hadris(?:-[a-z0-9]+)*)/([^/#)\s]+)", contents)
        references.extend(re.findall(
            r"cargo (?:install|binstall) (hadris(?:-[a-z0-9]+)*) --version ([^\s`]+)", contents,
        ))
        for match in re.finditer(r"```toml\n(.*?)```", contents, re.DOTALL):
            try:
                recipe = tomllib.loads(match.group(1))
            except tomllib.TOMLDecodeError:
                continue
            tables = [recipe, *recipe.get("target", {}).values(), recipe.get("workspace", {})]
            for table in tables:
                for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                    for name, dependency in table.get(kind, {}).items():
                        if isinstance(dependency, dict):
                            name = dependency.get("package", name)
                            dependency = dependency.get("version")
                        if isinstance(dependency, str):
                            references.append((name, dependency.lstrip("=^")))
        for name, version in references:
            if name in versions and version not in (versions[name], "latest"):
                errors.append(
                    f"{source.relative_to(root)}: {name} references {version}, expected {versions[name]}"
                )
    return errors


def self_test() -> int:
    with tempfile.TemporaryDirectory() as directory:
        temporary_root = Path(directory).resolve()
        root = temporary_root / "repository"
        docs = root / "docs"
        docs.mkdir(parents=True)
        (temporary_root / "outside.md").write_text("outside\n")
        (docs / "target.md").write_text("target\n")
        source = docs / "source.md"
        source.write_text("[valid](target.md)\n[escape](../../outside.md)\n")
        errors = check_links([source], root)
        source.write_text("```toml\n[dependencies]\nhadris = \"3\"\nhadris = \"2\"\n```\n")
        toml_errors = check_toml([source], root)
        source.write_text('```toml\n[dependencies]\nhadris = "3.0.0-rc.1"\n```\n')
        manifest = root / "crates/core/hadris/Cargo.toml"
        manifest.parent.mkdir(parents=True)
        manifest.write_text('[package]\nname = "hadris"\nversion = "3.0.0-rc.2"\n')
        cli_manifest = root / "crates/tools/hadris-cli/Cargo.toml"
        cli_manifest.parent.mkdir(parents=True)
        cli_manifest.write_text('[package]\nname = "hadris-cli"\nversion = "3.0.0-rc.2"\n')
        readme = root / "README.md"
        readme.write_text(
            '```toml\n[dependencies.alias]\npackage = "hadris"\nversion = "3.0.0-rc.1"\n```\n'
            '[API](https://docs.rs/hadris/3.0.0-rc.1/hadris/)\n'
            '```sh\ncargo binstall hadris-cli --version 3.0.0-rc.1\n```\n'
        )
        version_errors = check_current_versions([readme, source], root)
        readme.write_text(readme.read_text().replace("3.0.0-rc.1", "3.0.0-rc.2"))
        current_errors = check_current_versions([readme], root)
    expected = [
        "docs/source.md:2: link target escapes repository ../../outside.md"
    ]
    if errors != expected or len(toml_errors) != 1 or len(version_errors) != 3 or current_errors:
        print(
            f"self-test failed: links={errors!r}, TOML={toml_errors!r}, "
            f"versions={version_errors!r}, current={current_errors!r}", file=sys.stderr,
        )
        return 1
    print("documentation checker self-test passed")
    return 0


def check_package_readmes() -> list[str]:
    errors = []
    for manifest in sorted((ROOT / "crates").glob("**/Cargo.toml")):
        package = tomllib.loads(manifest.read_text()).get("package", {})
        readme_value = package.get("readme")
        if not isinstance(readme_value, str):
            continue
        readme = (manifest.parent / readme_value).resolve()
        if not readme.exists():
            errors.append(f"{manifest.relative_to(ROOT)}: package README does not exist")
            continue
        if readme == ROOT / "README.md":
            continue
        contents = readme.read_text()
        relative = readme.relative_to(ROOT)
        if "## Documentation" not in contents:
            errors.append(f"{relative}: missing Documentation section")
        if "LICENSE-MIT" not in contents:
            errors.append(f"{relative}: missing MIT license link")
    return errors


def check_active_names(files: list[Path]) -> list[str]:
    errors = []
    stale_paths = ("release-candidate.md", "read-and-create-iso.md")
    for source in files:
        contents = source.read_text()
        for stale in stale_paths:
            if stale in contents:
                errors.append(f"{source.relative_to(ROOT)}: stale documentation path {stale}")
    if (ROOT / "website" / "docs" / "release-candidate.md").exists():
        errors.append("website/docs/release-candidate.md: use stability.md")
    return errors


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if sys.argv[1:]:
        print("usage: check-docs.py [--self-test]", file=sys.stderr)
        return 2
    files = documentation_files()
    errors = check_links(files)
    errors.extend(check_package_readmes())
    errors.extend(check_active_names(files))
    errors.extend(check_toml(files))
    errors.extend(check_current_versions(files))
    if errors:
        print("documentation checks failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"checked {len(files)} documentation files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

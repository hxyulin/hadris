// Generates Docusaurus version snapshots from the release tags.
// The output is not committed; see the website section of CONTRIBUTING.md.

import {execFileSync} from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import {fileURLToPath} from "node:url";

const website = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const generated = {
  docs: path.join(website, "versioned_docs"),
  sidebars: path.join(website, "versioned_sidebars"),
  static: path.join(website, "versioned_static"),
  versions: path.join(website, "versions.json"),
};
const RELEASE_TAG = /^v(\d+)\.(\d+)\.(\d+)$/;

function git(args) {
  return execFileSync("git", args, {
    cwd: website,
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function fail(message) {
  console.error(`sync-versions: ${message}`);
  process.exit(1);
}

function treeFiles(tag, dir) {
  return git(["ls-tree", "-r", "-z", "--full-tree", tag, "--", dir])
    .toString()
    .split("\0")
    .filter(Boolean)
    .map((line) => {
      const [meta, file] = line.split("\t");
      const [, type, object] = meta.split(" ");
      return {type, object, file};
    })
    .filter((entry) => entry.type === "blob");
}

// Copies the blobs under `dir` at `tag` into `destination`, except those
// for which `skip(relativePath)` is true. Returns the copied relative paths.
function exportTree(tag, dir, destination, skip = () => false) {
  const copied = [];
  for (const {object, file} of treeFiles(tag, dir)) {
    const relative = path.posix.relative(dir, file);
    if (skip(relative)) continue;
    const target = path.join(destination, ...relative.split("/"));
    fs.mkdirSync(path.dirname(target), {recursive: true});
    fs.writeFileSync(target, git(["cat-file", "blob", object]));
    copied.push(relative);
  }
  return copied;
}

function loadSidebars(tag) {
  const source = git(["show", `${tag}:website/sidebars.js`]).toString();
  const module = {exports: {}};
  vm.runInNewContext(source, {module, exports: module.exports, process: {env: {}}});
  return module.exports;
}

function releases() {
  if (git(["rev-parse", "--is-shallow-repository"]).toString().trim() === "true") {
    fail("the repository is a shallow clone; fetch full history with `git fetch --unshallow --tags`");
  }
  const latest = new Map();
  for (const tag of git(["tag", "--list", "v*"]).toString().split("\n")) {
    const match = RELEASE_TAG.exec(tag.trim());
    if (!match) continue;
    const [major, minor, patch] = match.slice(1).map(Number);
    const name = `${major}.${minor}`;
    const previous = latest.get(name);
    if (!previous || previous.patch < patch) {
      latest.set(name, {tag: match[0], name, major, minor, patch});
    }
  }
  if (latest.size === 0) {
    fail("no release tags (vX.Y.Z) found; fetch them with `git fetch --tags`");
  }
  return [...latest.values()]
    .filter(({tag}) => treeFiles(tag, "website/docs").length > 0)
    .filter(({tag}) => treeFiles(tag, "website/sidebars.js").length > 0)
    .sort((a, b) => b.major - a.major || b.minor - a.minor);
}

const versions = releases();
for (const target of Object.values(generated)) {
  fs.rmSync(target, {recursive: true, force: true});
}

const currentStatic = path.join(website, "static");
const exists = (root, relative) => fs.existsSync(path.join(root, ...relative.split("/")));

for (const release of versions) {
  const version = `${release.major}.${release.minor}.${release.patch}`;
  exportTree(release.tag, "website/docs", path.join(generated.docs, `version-${release.name}`));

  const sidebars = loadSidebars(release.tag);
  const first = Object.keys(sidebars)[0];
  sidebars[first] = [
    ...sidebars[first],
    {type: "link", label: "API reference", href: `https://docs.rs/hadris/${version}/hadris/`},
  ];
  fs.mkdirSync(generated.sidebars, {recursive: true});
  fs.writeFileSync(
    path.join(generated.sidebars, `version-${release.name}-sidebars.json`),
    `${JSON.stringify(sidebars, null, 2)}\n`,
  );

  const kept = exportTree(
    release.tag,
    "website/static",
    generated.static,
    (relative) => exists(currentStatic, relative) || exists(generated.static, relative),
  );
  console.log(`${release.tag} -> version ${release.name}`);
  for (const file of kept) console.log(`  kept static/${file} from ${release.tag}`);
}

fs.writeFileSync(
  generated.versions,
  `${JSON.stringify(versions.map(({name}) => name), null, 2)}\n`,
);

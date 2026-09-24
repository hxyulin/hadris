---
title: Contributing
---

# Contributing

See the repository's
[contribution guide](https://github.com/hxyulin/hadris/blob/next/CONTRIBUTING.md)
for the Rust toolchain, feature checks, API snapshots, specification annotations,
and pull-request workflow.

For documentation changes:

```bash
python3 scripts/check-docs.py
cd website
npm ci
npm start
```

Run `npm run build` before submitting a pull request.

The site is versioned. `website/docs` holds the unreleased docs, served
under `next/`. The docs of each released minor are generated from its
newest `vX.Y.Z` tag and are not committed; the newest release is served at
the root. To build the site with every version, from a clone with full
history and tags:

```bash
cd website
npm run versions
npm run build
```

`npm run versions` rewrites `versions.json`, `versioned_docs/` and
`versioned_sidebars/`; delete them to go back to a build of the current
docs only. A release rebuilds the deployed site, so a new minor appears
without further changes.

## Documentation responsibilities

- The root README introduces the project and its architecture.
- The documentation site owns concepts, crate selection, and task-oriented
  workflows.
- Crate READMEs explain package-specific features, minimum configurations, and
  a quick start.
- Rustdoc owns detailed API contracts and examples tied to individual items.

Prefer linking between these layers instead of copying long sections. New
website examples should come from a compiled workspace example when practical;
otherwise verify the exact feature combination and API names before publishing.
When Cargo features change, update the capability matrix and affected crate
README in the same pull request.

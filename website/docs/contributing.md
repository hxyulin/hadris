---
title: Contributing
---

# Contributing

See the repository's
[contribution guide](https://github.com/hxyulin/hadris/blob/main/CONTRIBUTING.md)
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

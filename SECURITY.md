# Security Policy

## Supported Versions

| Line | Branch | Security fixes |
|---|---|---|
| 3.x | `main` | The latest release of each crate; during the 3.0 release candidates, the latest candidate |
| 2.5.x | `v2` | While the V2 line is maintained; the end date will be announced here and in the README |
| 2.4 and earlier, 1.x | None | Not supported; upgrade to 2.5.x or 3.x |

After 3.0.0 each crate versions on its own, so a fix ships as a patch
release of the affected crate. The previews `hadris-ntfs` and `hadris-apfs`
receive fixes in their latest 3.y release only, and the raw crates in
their latest 0.x release. Fixed vulnerabilities are published as GitHub
security advisories and submitted to the RustSec advisory database.

## Reporting a Vulnerability

Please report security vulnerabilities **privately**, not in public issues or
pull requests.

Use GitHub's [private vulnerability reporting][pvr]: open the
[Security tab](https://github.com/hxyulin/hadris/security) and click
**Report a vulnerability**. This keeps the details confidential until a fix is
released and lets us coordinate a security advisory.

Please include:

- affected crate and version (e.g. `hadris-cpio 3.0.0-rc.1`)
- a description of the impact (e.g. unbounded allocation / DoS)
- a minimal reproducer or proof-of-concept
- the target environment where it bites (e.g. no-std / initramfs)

We aim to acknowledge reports within a few days.

[pvr]: https://docs.github.com/en/code-security/security-advisories/working-with-repository-security-advisories/configuring-private-vulnerability-reporting-for-a-repository

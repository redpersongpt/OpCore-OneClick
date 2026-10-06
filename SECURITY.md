# Security Policy

## Supported Versions

Security fixes go into the latest release line (currently 6.x) on `main`.
Older releases do not receive patches.

## Reporting a Vulnerability

If you found a security issue, please report it privately.

Preferred route:

- Open a [GitHub Security Advisory draft](https://github.com/redpersongpt/OpCore-OneClick/security/advisories/new) for this repository

If that is not available, contact the maintainer through GitHub and include:

- a short description of the issue
- affected version or commit
- reproduction steps
- impact
- any suggested mitigation

Please do not post working exploit details in a public issue before the problem has been reviewed.

## What Counts as a Security Issue

Examples include:

- a disk-write path that can reach a disk the user did not confirm, an
  internal disk or the system disk
- bypassing, replaying or forging the flash confirmation token
- a download that is used without its SHA-256 (or, for recovery images, its
  chunklist) being verified
- path traversal or arbitrary file write, for example through archive
  extraction, profile import or export paths
- command injection into a child process or an elevated script
- a webview page or link that can call backend commands or open non-https URLs
- remote code execution
- shipping secrets or credentials

How the app protects these paths is described under "Safety model" in
[docs/architecture-map.md](docs/architecture-map.md).

## Verifying Downloads

Releases from 6.0.0 on include a `SHA256SUMS.txt` and a build provenance attestation:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
gh attestation verify <downloaded file> -R redpersongpt/OpCore-OneClick
```

## Response Expectations

The project is maintained on a best-effort basis, but valid reports will be reviewed as quickly as practical.

When a report is confirmed, the likely path is:

1. reproduce and scope the issue
2. prepare a fix
3. publish the patch
4. credit the reporter if they want public credit

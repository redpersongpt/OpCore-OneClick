# Releasing

Releases are built and published by `.github/workflows/release.yml`. Pushing a
tag that starts with `v` starts it; it can also be run by hand from the Actions
tab for an existing tag.

## Checklist

1. Bump the version everywhere it lives and check it:

   ```bash
   # package.json and src-tauri/tauri.conf.json: "version": "6.0.0"
   # src-tauri/Cargo.toml:                       version = "6.0.0"
   npm install --package-lock-only        # package-lock.json
   (cd src-tauri && cargo check)          # Cargo.lock
   node scripts/check-version.mjs v6.0.0
   ```

2. Turn the `## 6.0.0 — unreleased` heading in `CHANGELOG.md` into
   `## 6.0.0 — YYYY-MM-DD`. The release notes are taken from that section
   (`scripts/changelog-notes.mjs`).
3. Make sure CI is green on `main`.
4. Tag and push:

   ```bash
   git tag -a v6.0.0 -m "v6.0.0"
   git push origin v6.0.0
   ```

## What the workflow does

| Job | Runner | Output |
|---|---|---|
| Verify | ubuntu-24.04 | Tag exists, versions match the tag, `tsc`, `vitest` |
| Build (linux) | ubuntu-22.04 | `.AppImage`, `.deb` (built on 22.04 so they run on glibc 2.35 and newer) |
| Build (windows) | windows-latest | NSIS `*-setup.exe` |
| Build (macos) | macos-latest | Universal (`aarch64` + `x86_64`) `.dmg` |
| Publish | ubuntu-24.04 | `SHA256SUMS.txt`, build provenance attestation, GitHub release |

- Tags with a `-` (for example `v6.1.0-beta.1`) are published as pre-releases;
  all others are marked as the latest release.
- Publishing is safe to repeat. If the release already exists its notes and
  flags are left alone and assets with the same file name are replaced, so a
  failed run can simply be re-run.
- Manual runs build the tag you enter, never the branch the run was started
  from, and stop if the tag does not exist.

Users can check a download with the checksums file, or with the attestation:

```bash
gh attestation verify OpCore-OneClick_6.0.0_amd64.AppImage -R redpersongpt/OpCore-OneClick
```

## Code signing (optional)

Without any signing secrets the workflow still produces working bundles:

- the Windows installer is unsigned, so SmartScreen shows "Windows protected
  your PC" until the user picks **More info → Run anyway**;
- the macOS app is ad-hoc signed (`bundle.macOS.signingIdentity` is `"-"`), so
  on first launch the user has to allow it in **System Settings → Privacy &
  Security → Open Anyway**.

### macOS: Developer ID signing and notarization

Add these repository secrets (Settings → Secrets and variables → Actions). The
build step only uses them when `APPLE_CERTIFICATE` and `APPLE_SIGNING_IDENTITY`
are both set, and notarizes only when all three Apple ID secrets are set.

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | Base64 of the exported "Developer ID Application" `.p12` (`base64 -i cert.p12`) |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | For example `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | Apple account e-mail (notarization) |
| `APPLE_PASSWORD` | App-specific password for that account (notarization) |
| `APPLE_TEAM_ID` | Team ID from the Apple Developer account (notarization) |

`APPLE_SIGNING_IDENTITY` from the environment takes precedence over the `"-"`
in `tauri.conf.json`. See <https://v2.tauri.app/distribute/sign/macos/>.

### Windows

Windows signing is configured in `tauri.conf.json` (`bundle.windows`), so it
needs a small config change in addition to secrets. Two common routes:

- **Azure Artifact Signing** (formerly Trusted Signing): set
  `"signCommand": "artifact-signing-cli -e https://<region>.codesigning.azure.net -a <account> -c <profile> -d OpCore-OneClick %1"`,
  install that CLI in the Windows build job as the Tauri guide describes, and
  pass `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` and `AZURE_TENANT_ID` as
  environment variables to the build step.
- **Certificate (`.pfx`)**: store it as `WINDOWS_CERTIFICATE` (base64) and
  `WINDOWS_CERTIFICATE_PASSWORD`, import it into `Cert:\CurrentUser\My` in a
  step before the build, and set `certificateThumbprint`,
  `"digestAlgorithm": "sha256"` and a `timestampUrl`.

See <https://v2.tauri.app/distribute/sign/windows/>. Free signing for open
source projects is available from SignPath Foundation.

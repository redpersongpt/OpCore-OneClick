# Contributing

Thanks for taking the time to improve OpCore-OneClick.

## Before You Start

- Read the [README](README.md) and [docs/architecture-map.md](docs/architecture-map.md)
- Search existing issues and pull requests before opening a new one
- Keep changes focused; unrelated cleanup should be a separate pull request

## What We Accept

- Bug fixes
- Hardware support backed by evidence (a working EFI, a Dortania reference,
  upstream kext documentation, or a boot log)
- Corrections to the knowledge bases (CPU, GPU, chipset, codec, device and
  SMBIOS tables) with a source
- Stability and UI improvements that keep the existing workflow
- Clear documentation improvements

## What Needs Extra Care

Changes in these areas need especially strong validation:

- Disk listing and flashing (`platform/*/disk.rs`, `commands/disk.rs`, `safety/`)
- Hardware detection (`platform/*/scanner.rs`, `domain/profile.rs`)
- The build plan and config generation (`domain/planner.rs`,
  `domain/config_writer.rs`, `domain/acpi/`)
- Pinned downloads (`domain/kext_catalog.rs`): every pin needs the exact
  upstream URL and its SHA-256
- Recovery download and caching

If your change touches one of those paths, include a short note about what
changed, what hardware or scenario it targets, and how you validated it.

Third-party binaries (kexts, OpenCore, OcBinaryData) are never committed. The
app downloads them from upstream at build time and verifies them.

## Development Setup

Prerequisites:

- Node.js 22 LTS (20.19 or newer works)
- Rust stable through [rustup](https://rustup.rs), with `clippy`
- Platform packages from the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):
  - **Windows:** Microsoft C++ Build Tools and WebView2 (preinstalled on
    Windows 10/11)
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Debian/Ubuntu:**

    ```bash
    sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
      libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
    ```

Then:

```bash
npm ci
npm run tauri:dev
```

`tauri dev` starts the Vite dev server itself (`beforeDevCommand`), so do not
run `npm run dev` in another terminal at the same time: the second server
cannot bind port 5173.

The Windows executable asks for administrator rights (writing USB drives
needs them), so on Windows start the terminal as administrator before
`npm run tauri:dev`. On Linux and macOS the app runs as your user and asks
for elevation (`pkexec` or the macOS password prompt) only when it writes the
USB drive. On Linux the firmware ACPI tables are readable by root only, so a
scan from a normal session skips them with a warning. Use a USB drive you do
not care about when testing the flash path.

A release build of the installers for your OS:

```bash
npm run tauri:build
```

## Tests and Checks

Run these before opening a pull request. CI runs the same checks on Linux,
Windows and macOS.

```bash
npx tsc --noEmit
npx vitest run
npm run build

cd src-tauri
cargo clippy --all-targets -- -D warnings
cargo test
```

- Unit tests live next to the code (`#[cfg(test)] mod tests`) or in
  `src-tauri/tests/`. Tests must not need network access; mark the few that do
  with `#[ignore]` and run them with `cargo test -- --ignored`.
- `cargo test` regenerates the TypeScript bindings in `src/bridge/generated`
  from the Rust IPC types. If you changed a type in `contracts.rs` or
  `domain/model.rs`, commit the regenerated files; CI fails when they are out
  of date.

### Checking the Windows build from macOS or Linux

The Windows scanner and disk code sits behind `cfg(windows)`, so a normal
build on macOS or Linux never compiles it. You can type-check it without a
Windows machine:

```bash
scripts/check-windows.sh          # cargo check for x86_64-pc-windows-msvc
scripts/check-windows.sh clippy   # clippy with -D warnings
```

The script adds the Rust target if needed. tauri-build compiles the Windows
resource file with `llvm-rc`; when it is not installed the script uses a
stand-in that writes an empty resource file, which is enough for checking
(linking a real Windows executable still needs Windows or a full cross
toolchain). Linux-only code is checked the same way by CI on Ubuntu.

## Pull Request Guidelines

- Explain the problem first, then the fix
- Include screenshots for UI changes
- Include logs or error text for bug fixes when possible
- Do not bundle unrelated refactors into the same PR
- Keep generated output other than `src/bridge/generated`, local build output
  and personal notes out of the diff

## Commit Style

There is no strict commit format, but good commits are:

- small enough to review
- specific about the change
- free of unrelated noise

Examples:

- `fix: preserve AMD core count in EFI build profile`
- `docs: add security policy`
- `ui: add target macOS selector to compatibility step`

## Reporting Compatibility Problems

For hardware-specific issues, include as much of this as you can:

- CPU model
- GPU model(s)
- Motherboard or laptop model
- Wi-Fi / Ethernet chipset
- Target macOS version
- What step failed
- The support log from Settings → Export support log, or a profile
  exported from the Hardware step

## Releases

Maintainers: see [docs/releasing.md](docs/releasing.md).

## Security Issues

Please do not open public issues for vulnerabilities that could put users or
data at risk. Follow [SECURITY.md](SECURITY.md) instead.

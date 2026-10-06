#!/usr/bin/env bash
# Type-checks the Windows build from macOS or Linux.
#
#   scripts/check-windows.sh          # cargo check --all-targets
#   scripts/check-windows.sh clippy   # cargo clippy --all-targets -- -D warnings
#
# Every cfg(windows) module is compiled for x86_64-pc-windows-msvc. Nothing is
# linked, so no executable comes out of this. tauri-build compiles the Windows
# resource file with llvm-rc; if llvm-rc is not installed, a stand-in that
# writes an empty .res file is put on PATH, which is enough for a check build.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
target="x86_64-pc-windows-msvc"
mode="${1:-check}"

if ! rustup target list --installed | grep -qx "$target"; then
  rustup target add "$target"
fi

if ! command -v llvm-rc > /dev/null 2>&1; then
  stub_dir="$(mktemp -d)"
  trap 'rm -rf "$stub_dir"' EXIT
  cat > "$stub_dir/llvm-rc" << 'STUB'
#!/usr/bin/env bash
prev=""
for arg in "$@"; do
  case "$prev" in /fo | -fo | /FO | -FO) : > "$arg" ;; esac
  case "$arg" in /fo?* | -fo?* | /FO?* | -FO?*) : > "${arg:3}" ;; esac
  prev="$arg"
done
STUB
  chmod +x "$stub_dir/llvm-rc"
  export PATH="$stub_dir:$PATH"
fi

cd "$root/src-tauri"
case "$mode" in
  check) cargo check --target "$target" --all-targets ;;
  clippy) cargo clippy --target "$target" --all-targets -- -D warnings ;;
  *)
    echo "usage: $0 [check|clippy]" >&2
    exit 2
    ;;
esac

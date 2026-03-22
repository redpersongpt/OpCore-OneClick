#!/usr/bin/env bash
set -euo pipefail

# Downloads core kexts for embedding into the app as offline fallback.
# Run this script to update the embedded kexts to latest versions.

KEXT_DIR="$(cd "$(dirname "$0")/.." && pwd)/electron/assets/kexts"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$KEXT_DIR"

# Core kexts to embed — repo, asset filter, kext names to extract
REPOS=(
  "acidanthera/Lilu|RELEASE|Lilu.kext"
  "acidanthera/VirtualSMC|RELEASE|VirtualSMC.kext,SMCBatteryManager.kext,SMCSuperIO.kext,SMCProcessor.kext"
  "acidanthera/WhateverGreen|RELEASE|WhateverGreen.kext"
  "acidanthera/AppleALC|RELEASE|AppleALC.kext"
  "acidanthera/RTCMemoryFixup|RELEASE|RTCMemoryFixup.kext"
  "acidanthera/VoodooPS2|RELEASE|VoodooPS2Controller.kext"
  "acidanthera/RestrictEvents|RELEASE|RestrictEvents.kext"
  "acidanthera/NVMeFix|RELEASE|NVMeFix.kext"
  "acidanthera/CPUTopologyRebuild|RELEASE|CPUTopologyRebuild.kext"
)

fetch_kext() {
  local spec="$1"
  IFS='|' read -r repo filter kext_names <<< "$spec"

  echo "Fetching $repo..."

  # Get latest release asset URL
  local api_url="https://api.github.com/repos/$repo/releases/latest"
  local release_json
  release_json=$(curl -sL -H "User-Agent: OpCore-OneClick/1.0" "$api_url")

  # Find matching zip asset
  local asset_url
  if [[ -n "$filter" ]]; then
    asset_url=$(echo "$release_json" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a['name'].endswith('.zip') and '${filter}'.upper() in a['name'].upper():
        print(a['browser_download_url']); break
" 2>/dev/null || true)
  fi

  if [[ -z "$asset_url" ]]; then
    asset_url=$(echo "$release_json" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if a['name'].endswith('.zip'):
        print(a['browser_download_url']); break
" 2>/dev/null || true)
  fi

  if [[ -z "$asset_url" ]]; then
    echo "  WARN: No asset found for $repo"
    return
  fi

  local version
  version=$(echo "$release_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tag_name','unknown'))" 2>/dev/null || echo "unknown")

  # Download
  local zip_file="$TMP_DIR/$(echo "$repo" | tr '/' '_').zip"
  curl -sL -o "$zip_file" "$asset_url"

  # Extract
  local extract_dir="$TMP_DIR/extract_$(echo "$repo" | tr '/' '_')"
  mkdir -p "$extract_dir"
  unzip -qo "$zip_file" -d "$extract_dir"

  # Find and copy each kext
  IFS=',' read -ra KEXTS <<< "$kext_names"
  for kext in "${KEXTS[@]}"; do
    local found
    found=$(find "$extract_dir" -type d -name "$kext" | head -1)
    if [[ -n "$found" ]]; then
      rm -rf "$KEXT_DIR/$kext"
      cp -R "$found" "$KEXT_DIR/$kext"
      # Write version marker outside bundle (avoids macOS codesign issues)
      local base="${kext%.kext}"
      echo "$version" > "$KEXT_DIR/${base}.version"
      echo "  $kext $version"
    else
      echo "  WARN: $kext not found in archive"
    fi
  done
}

echo "Downloading embedded kexts to $KEXT_DIR"
echo "========================================="

for spec in "${REPOS[@]}"; do
  fetch_kext "$spec"
done

echo ""
echo "Done. Embedded kexts:"
ls -1 "$KEXT_DIR"

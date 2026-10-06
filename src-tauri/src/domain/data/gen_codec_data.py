#!/usr/bin/env python3
"""Regenerate the HD Audio codec tables embedded by `domain/codec_db.rs`.

Usage:
    gen_codec_data.py <AppleALC checkout> <linux checkout or sound/hda/codecs copy>

Inputs:
  * AppleALC `Resources/<Codec>/Info.plist` at the release tag that the kext
    catalog pins (CodecID, Vendor, CodecName, Files.Layouts[].Id/Comment).
  * Linux `sound/hda/codecs/**/*.c` (HDA_CODEC_ENTRY / HDA_CODEC_ID* macros),
    used only for the names of codecs AppleALC does not support.

Outputs (next to this script):
  * applealc_codecs.json  - {"source": ..., "codecs": [{id, vendor, name, layouts: [[id, comment], ...]}]}
  * hda_codec_names.json  - {"source": ..., "names": [[id, name], ...]}
"""

import glob
import json
import os
import plistlib
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
APPLEALC_TAG = "1.9.8"

VENDOR_IDS = {
    "AMD": 0x1002,
    "AMDZEN": 0x1022,
    "AnalogDevices": 0x11D4,
    "CirrusLogic": 0x1013,
    "Conexant": 0x14F1,
    "Creative": 0x1102,
    "IDT": 0x111D,
    "Intel": 0x8086,
    "NVIDIA": 0x10DE,
    "Realtek": 0x10EC,
    "SigmaTel": 0x8384,
    "VIA": 0x1106,
}

VENDOR_DISPLAY = {
    "AnalogDevices": "Analog Devices",
    "CirrusLogic": "Cirrus Logic",
}


def clean_name(directory, codec_name, vendor):
    name = codec_name.strip()
    if name.lower().startswith("0x"):
        name = directory
    name = re.sub(r"\s*\(.*\)$", "", name)
    if vendor == "IDT" and name.startswith("IDT"):
        name = name[3:]
    return name.replace("_", "/")


def applealc(root):
    codecs = {}
    for plist_path in sorted(glob.glob(os.path.join(root, "Resources", "*", "Info.plist"))):
        directory = os.path.basename(os.path.dirname(plist_path))
        with open(plist_path, "rb") as f:
            info = plistlib.load(f)
        if "CodecID" not in info or "Vendor" not in info:
            continue
        vendor = info["Vendor"]
        codec_id = (VENDOR_IDS[vendor] << 16) | (info["CodecID"] & 0xFFFF)
        layouts = {}
        for layout in info.get("Files", {}).get("Layouts", []):
            comment = " ".join((layout.get("Comment") or "").split())
            layouts[int(layout["Id"])] = comment
        name = clean_name(directory, info.get("CodecName", directory), vendor)
        entry = codecs.get(codec_id)
        if entry is None:
            codecs[codec_id] = {
                "id": f"{codec_id:08X}",
                "vendor": VENDOR_DISPLAY.get(vendor, vendor),
                "name": name,
                "layouts": layouts,
            }
        else:
            # Several resource folders can share one codec id (IDT 92HD81B1X5 / 92HD87B1).
            if name not in entry["name"].split("/"):
                entry["name"] = f"{entry['name']}/{name}"
            for layout_id, comment in layouts.items():
                if not entry["layouts"].get(layout_id):
                    entry["layouts"][layout_id] = comment
    out = []
    for codec_id in sorted(codecs):
        entry = codecs[codec_id]
        entry["layouts"] = [[k, v] for k, v in sorted(entry["layouts"].items())]
        out.append(entry)
    return out


ENTRY = re.compile(
    r"HDA_CODEC_(?:ENTRY|ID|ID_MODEL|ID_REV|ID_REV_MODEL)(?:_REV)?\(\s*(0x[0-9a-fA-F]+)\s*,(\s*0x[0-9a-fA-F]+\s*,)?\s*\"([^\"]+)\""
)
SKIP_VENDORS = {0x10DE, 0x1AF4}


def linux_names(root):
    # Revision-specific entries ("ALC660" for one 0x10ec0861 revision) are
    # only used when the codec has no plain entry.
    plain = {}
    by_rev = {}
    base = os.path.join(root, "sound", "hda", "codecs") if os.path.isdir(os.path.join(root, "sound")) else root
    for path in sorted(glob.glob(os.path.join(base, "**", "*.c"), recursive=True)):
        with open(path, encoding="utf-8", errors="replace") as f:
            text = f.read()
        for m in ENTRY.finditer(text):
            codec_id = int(m.group(1), 16)
            if codec_id >> 16 in SKIP_VENDORS:
                continue
            name = re.sub(r"\s*\(.*\)$", "", m.group(3)).strip()
            name = re.sub(r" rev\d+$", "", name)
            (by_rev if m.group(2) else plain).setdefault(codec_id, set()).add(name)
    out = []
    for codec_id in sorted(set(plain) | set(by_rev)):
        candidates = sorted(plain.get(codec_id) or by_rev[codec_id], key=lambda n: (len(n), n))
        out.append([f"{codec_id:08X}", candidates[0]])
    return out


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    codecs = applealc(sys.argv[1])
    with open(os.path.join(HERE, "applealc_codecs.json"), "w", encoding="utf-8") as f:
        json.dump(
            {"source": f"AppleALC {APPLEALC_TAG} Resources/*/Info.plist", "codecs": codecs},
            f,
            ensure_ascii=False,
            separators=(",", ":"),
        )
        f.write("\n")
    names = linux_names(sys.argv[2])
    with open(os.path.join(HERE, "hda_codec_names.json"), "w", encoding="utf-8") as f:
        json.dump(
            {"source": "Linux sound/hda/codecs HDA_CODEC_ENTRY tables", "names": names},
            f,
            ensure_ascii=False,
            separators=(",", ":"),
        )
        f.write("\n")
    print(f"{len(codecs)} AppleALC codecs, {len(names)} named codec ids")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""The license texts of the launcher's Rust libraries, as each crate ships them.

    cargo metadata --format-version 1 --manifest-path coop-launcher/Cargo.toml | crate_licenses.py > out.txt
"""
import glob
import json
import os
import sys

PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")

meta = json.load(sys.stdin)
missing = []
standard = {}  # the first MIT / Apache-2.0 text found, for the crates that ship none
for p in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
    if p["source"] is None:
        continue  # the launcher itself
    d = os.path.dirname(p["manifest_path"])
    files = sorted(f for f in glob.glob(os.path.join(d, "*"))
                   if os.path.isfile(f) and os.path.basename(f).upper().startswith(PREFIXES))
    print("=" * 78)
    print(f"{p['name']} {p['version']}: {p.get('license') or 'see below'}")
    if not files:
        missing.append(p["name"])
        print("(no license file in the crate; its license is the one named above)")
    for f in files:
        with open(f, encoding="utf-8", errors="replace") as fh:
            text = fh.read().strip()
        print(f"--- {os.path.basename(f)}")
        print(text)
        upper = os.path.basename(f).upper()
        if "APACHE" in upper and "Apache License" in text:
            standard.setdefault("Apache-2.0", text)
        elif "MIT" in upper and "Permission is hereby granted" in text:
            standard.setdefault("MIT", text)
    print()
if missing:
    print("=" * 78)
    print("The standard license texts, for the crates above that name a license but ship no file")
    print("(their copyright holders are the authors listed in each crate):")
    for name in ("MIT", "Apache-2.0"):
        if name in standard:
            print(f"--- {name}")
            print(standard[name])
            print()
if missing:
    print(f"[*] {len(missing)} crates ship no license file: the standard texts are appended", file=sys.stderr)

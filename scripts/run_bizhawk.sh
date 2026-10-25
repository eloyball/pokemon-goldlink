#!/usr/bin/env bash
# Launches BizHawk with the co-op ROM and opens the Co-op Bridge.
#
# Every instance shares the one BizHawk install, BizHawk/ExternalTools/
# CoopBridge.dll and hg-engine/test.nds, so a rebuild reaches all of them at
# once. Each named instance gets its own BizHawk config, SaveRAM, savestates
# and bridge identity (coop_config.json, which holds the player secret) under
# instances/<name>/, so instances never overwrite each other's saves.
#
#   scripts/run_bizhawk.sh        # default instance: BizHawk/config.ini and its saves
#   scripts/run_bizhawk.sh p2     # another player: instances/p2/
#
# Instances can run at the same time; start one per player.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIZHAWK="$ROOT/BizHawk"
ROM="$ROOT/hg-engine/test.nds"
NAME="${1:-}"

if [[ ! -f "$ROM" ]]; then
    echo "[-] $ROM not found; run ./build_all.sh first." >&2
    exit 1
fi

args=(--open-ext-tool-dll=CoopBridge)

if [[ -n "$NAME" ]]; then
    if [[ ! "$NAME" =~ ^[A-Za-z0-9_-]+$ ]]; then
        echo "[-] Instance names may only contain letters, digits, '-' and '_'." >&2
        exit 1
    fi
    DIR="$ROOT/instances/$NAME"
    CFG="$DIR/config.ini"
    mkdir -p "$DIR/NDS"

    if [[ ! -f "$CFG" ]]; then
        # First run: start from the default config (input bindings, firmware,
        # display settings) but point the NDS base path (SaveRAM, savestates,
        # screenshots) at this instance.
        python3 - "$BIZHAWK/config.ini" "$CFG" "$DIR/NDS" <<'EOF'
import json
import sys

src, dst, nds_dir = sys.argv[1:4]
try:
    with open(src, encoding="utf-8") as f:
        cfg = json.load(f)
except FileNotFoundError:
    sys.exit(f"[-] {src} not found: launch the default instance once first.")

for entry in cfg["PathEntries"]["Paths"]:
    if entry.get("System") == "NDS" and entry.get("Type") == "Base":
        entry["Path"] = nds_dir
cfg["SingleInstanceMode"] = False

with open(dst, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2)
print(f"[+] Created {dst} (saves in {nds_dir})")
EOF
    fi
    args+=(--config="$CFG")
fi

exec "$BIZHAWK/EmuHawkMono.sh" "${args[@]}" "$ROM"

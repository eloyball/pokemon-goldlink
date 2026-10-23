#!/usr/bin/env python3
"""Extracts gCoopCommBlock's exact ARM9 address and Main RAM offset into coop_offsets.json.

Uses arm-none-eabi-nm on hg-engine/build/linked.o. Fails hard (exit 1) if the
symbol is missing or its address does not fall within Overlay 129
(0x023D8060..0x023E0000); no default offset is ever substituted.
"""
import json
import subprocess
import sys
from pathlib import Path

TARGET_SYMBOL = "gCoopCommBlock"
MAIN_RAM_MASK = 0x003FFFFF
OVERLAY_129_START = 0x023D8060
OVERLAY_129_END = 0x023E0000


def die(msg: str) -> None:
    print(f"[!] {msg}", file=sys.stderr)
    sys.exit(1)


def find_symbol_address(hg_root: Path) -> int:
    linked_o = hg_root / "build" / "linked.o"
    if not linked_o.is_file():
        die(
            f"{linked_o} not found; run 'make' in hg-engine to produce the "
            f"linked object before extracting {TARGET_SYMBOL}."
        )

    try:
        out = subprocess.check_output(
            ["arm-none-eabi-nm", str(linked_o)],
            text=True,
            stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as e:
        stderr_text = (e.stderr or b"").decode(errors="replace").strip()
        die(f"arm-none-eabi-nm failed on {linked_o}: {stderr_text}")
    except FileNotFoundError:
        die("arm-none-eabi-nm not found on PATH; install the arm-none-eabi toolchain.")

    matches = []
    for line in out.splitlines():
        parts = line.split()
        # arm-none-eabi-nm format: <address> <type> <symbol>
        if len(parts) >= 3 and parts[2] == TARGET_SYMBOL:
            try:
                matches.append((parts[1], int(parts[0], 16)))
            except ValueError:
                continue

    defined = [addr for sym_type, addr in matches if sym_type not in {"U", "u", "N", "n"}]
    if not defined:
        die(
            f"{TARGET_SYMBOL} is missing (or only referenced) in {linked_o}; "
            f"refusing to fall back to a default offset. Verify the coop "
            f"mailbox code is compiled and linked into the build."
        )

    in_range = [addr for addr in defined if OVERLAY_129_START <= addr < OVERLAY_129_END]
    if not in_range:
        shown = ", ".join(f"0x{addr:08X}" for addr in defined)
        die(
            f"{TARGET_SYMBOL} defined at {shown} in {linked_o}; no definition "
            f"falls within Overlay 129 (0x{OVERLAY_129_START:08X}..0x{OVERLAY_129_END:08X}); "
            f"refusing to write coop_offsets.json."
        )
    arm9_addr = in_range[0]

    print(f"[*] Found {TARGET_SYMBOL} in {linked_o} at 0x{arm9_addr:08X}")
    return arm9_addr


def main() -> None:
    repo_root = Path(__file__).resolve().parent.parent
    hg_root = repo_root / "hg-engine"
    arm9_addr = find_symbol_address(hg_root)
    main_ram_offset = arm9_addr & MAIN_RAM_MASK

    payload = {
        "symbol": TARGET_SYMBOL,
        "arm9_bus_address": f"0x{arm9_addr:08X}",
        "main_ram_offset": f"0x{main_ram_offset:06X}",
        "main_ram_offset_int": main_ram_offset,
        "mailbox_size": 512,
    }

    out_file = repo_root / "coop_offsets.json"
    out_file.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(
        f"[+] Wrote {out_file}: ARM9={payload['arm9_bus_address']} | Main RAM Offset={payload['main_ram_offset']}"
    )


if __name__ == "__main__":
    main()

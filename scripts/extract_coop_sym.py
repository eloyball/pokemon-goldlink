#!/usr/bin/env python3
"""Extracts gCoopCommBlock's exact ARM9 address and Main RAM offset into coop_offsets.json.

Uses arm-none-eabi-nm on hg-engine/build/linked.o. Fails hard (exit 1) if the
symbol is missing, is not exactly MAILBOX_SIZE bytes, does not fit entirely
within Overlay 129 (0x023D8060..0x023E0000), or is not at the pinned address
src/linker.ld places it at. No default offset is ever substituted.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

TARGET_SYMBOL = "gCoopCommBlock"
MAIN_RAM_MASK = 0x003FFFFF
OVERLAY_129_START = 0x023D8060
OVERLAY_129_END = 0x023E0000
# src/linker.ld places the .coop_mailbox section first in Overlay 129.
PINNED_ADDRESS = OVERLAY_129_START
MAILBOX_SIZE = 512
HEADER = Path("hg-engine/include/coop_sync.h")


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
            ["arm-none-eabi-nm", "-S", str(linked_o)],
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
        # arm-none-eabi-nm -S format: <address> <size> <type> <symbol>
        if len(parts) >= 4 and parts[3] == TARGET_SYMBOL:
            try:
                matches.append((parts[2], int(parts[0], 16), int(parts[1], 16)))
            except ValueError:
                continue

    defined = [(addr, size) for sym_type, addr, size in matches if sym_type not in {"U", "u", "N", "n"}]
    if not defined:
        die(
            f"{TARGET_SYMBOL} is missing (or only referenced) in {linked_o}; "
            f"refusing to fall back to a default offset. Verify the coop "
            f"mailbox code is compiled and linked into the build."
        )

    if len(defined) != 1:
        shown = ", ".join(f"0x{addr:08X}" for addr, _ in defined)
        die(f"{TARGET_SYMBOL} is defined {len(defined)} times ({shown}); expected exactly one.")
    arm9_addr, size = defined[0]

    if size != MAILBOX_SIZE:
        die(f"{TARGET_SYMBOL} is {size} bytes; expected {MAILBOX_SIZE}. Check include/coop_sync.h.")
    if not (OVERLAY_129_START <= arm9_addr and arm9_addr + size <= OVERLAY_129_END):
        die(
            f"{TARGET_SYMBOL} spans 0x{arm9_addr:08X}..0x{arm9_addr + size:08X}, outside Overlay 129 "
            f"(0x{OVERLAY_129_START:08X}..0x{OVERLAY_129_END:08X}); refusing to write coop_offsets.json."
        )
    if arm9_addr != PINNED_ADDRESS:
        die(
            f"{TARGET_SYMBOL} is at 0x{arm9_addr:08X}, not the pinned 0x{PINNED_ADDRESS:08X}. "
            f"Check that src/linker.ld keeps *(.coop_mailbox) first in .text."
        )

    print(f"[*] Found {TARGET_SYMBOL} in {linked_o} at 0x{arm9_addr:08X}")
    return arm9_addr


def read_mailbox_version(repo_root: Path) -> int:
    """COOP_MAILBOX_VERSION from coop_sync.h, the single source of truth."""
    header = repo_root / HEADER
    match = re.search(r"^#define\s+COOP_MAILBOX_VERSION\s+(\d+)", header.read_text(encoding="utf-8"), re.M)
    if not match:
        die(f"COOP_MAILBOX_VERSION not found in {header}.")
    return int(match.group(1))


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
        "mailbox_size": MAILBOX_SIZE,
        "mailbox_version": read_mailbox_version(repo_root),
    }

    out_file = repo_root / "coop_offsets.json"
    out_file.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(
        f"[+] Wrote {out_file}: ARM9={payload['arm9_bus_address']} | Main RAM Offset={payload['main_ram_offset']}"
    )


if __name__ == "__main__":
    main()

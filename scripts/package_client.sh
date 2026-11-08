#!/usr/bin/env bash
# The unified client's player release (dev/docs/unified-client/UC7.md): GoldLink-vX.Y-linux.tar.gz
# and GoldLink-vX.Y-windows.zip in build/client-release/vX.Y/, one program each (melonDS built
# in, no BizHawk, no .NET or Mono), built from HEAD with the debug tools off.
#
#   scripts/package_client.sh vX.Y [--publish]
#
#   --publish    also creates the GitHub release vX.Y as a draft. HEAD must be pushed.
#   NOTES=<file> the "Changes" section of the release notes (Markdown); a placeholder otherwise.
#   ALLOW_DIRTY=1  packages uncommitted changes (testing the script; never with --publish).
#   KEEP_RELEASE_BUILD=1  leaves hg-engine built without the debug tools.
#
# Needs: hg-engine/rom.nds (HeartGold USA), cargo with the x86_64-pc-windows-gnu target and
# mingw-w64-gcc, docker (the Linux program is built on Debian 11 for an old glibc), gh.
# macOS: built by .github/workflows/unified-client.yml (signing and notarization need the
# maintainer's Apple Developer ID; see UC7.md).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || { echo "usage: $0 vX.Y [--publish]" >&2; exit 2; }
PUBLISH=0
[[ "${2:-}" == "--publish" ]] && PUBLISH=1
NUMBER="${VERSION#v}"

OUT="$ROOT/build/client-release/$VERSION"
WORK="$OUT/work"
CACHE="$ROOT/build/release-cache"
REPOSITORY="eloyball/pokemon-goldlink"
ROM="$ROOT/hg-engine/rom.nds"
GAME="$ROOT/hg-engine/test.nds"
IMAGE="goldlink-client-build:bullseye" # rust:1.98.0-bullseye + CMake and the audio/input headers
MAX_GLIBC="2.31"

die() { echo "[-] $*" >&2; exit 1; }
step() { echo; echo "=== [$1/7] $2"; }

# ---------------------------------------------------------------------------
step 1 "Checks"
for tool in cargo docker x86_64-w64-mingw32-gcc arm-none-eabi-nm strings sha1sum sha256sum objdump; do
    command -v "$tool" > /dev/null || die "$tool is missing"
done
[[ $PUBLISH == 0 ]] || command -v gh > /dev/null || die "gh is missing"
rustup target list --installed | grep -qx x86_64-pc-windows-gnu || die "rustup target add x86_64-pc-windows-gnu"
[[ -f "$ROM" ]] || die "$ROM is missing (your HeartGold USA ROM)"
[[ -f crates/goldlink-core-sys/melonDS/CMakeLists.txt ]] || die "git submodule update --init (melonDS)"
COMMIT="$(git rev-parse HEAD)"
if ! git diff --quiet HEAD --; then
    [[ "${ALLOW_DIRTY:-0}" == 1 && $PUBLISH == 0 ]] || die "uncommitted changes: a release is built from a commit"
    echo "[!] ALLOW_DIRTY=1: built with uncommitted changes (a test package, never published)"
    DIRTY="-dirty"
fi
if [[ $PUBLISH == 1 ]]; then
    git fetch -q origin
    git branch -r --contains "$COMMIT" | grep -q . || die "push HEAD first (--publish tags it on GitHub)"
    gh release view "$VERSION" -R "$REPOSITORY" > /dev/null 2>&1 && die "the GitHub release $VERSION exists already"
fi
WIRE="$(grep -oP 'PROTOCOL_VERSION: u16 = \K[0-9]+' coop-relay/src/protocol.rs)"
MELONDS="$(git -C crates/goldlink-core-sys/melonDS describe --tags --exact-match 2> /dev/null || git -C crates/goldlink-core-sys/melonDS rev-parse --short HEAD)"
MAILBOX_OFFSET="$(python3 -I -c 'import json; print(json.load(open("coop_offsets.json"))["main_ram_offset_int"])')"
MAILBOX_VERSION="$(python3 -I -c 'import json; print(json.load(open("coop_offsets.json"))["mailbox_version"])')"
ROM_SHA1="$(sha1sum "$ROM" | cut -d' ' -f1)"
echo "[*] $VERSION from ${COMMIT:0:9}; relay protocol $WIRE; melonDS $MELONDS; ROM SHA-1 $ROM_SHA1"
rm -rf "$OUT"
mkdir -p "$WORK" "$CACHE"

# From here on hg-engine holds a release build: put the development build back on any exit.
restore_dev() {
    local status=$?
    if [[ "${KEEP_RELEASE_BUILD:-0}" == 1 ]]; then
        echo "[!] KEEP_RELEASE_BUILD=1: hg-engine/test.nds is the release build (no debug tools)"
        return $status
    fi
    echo; echo "=== Rebuilding the development ROM (debug tools on)"
    (cd "$ROOT/hg-engine" && make clean_code > /dev/null && make -j"$(nproc)" > "$OUT/dev-make.log" 2>&1) \
        && python3 "$ROOT/scripts/extract_coop_sym.py" > /dev/null \
        && echo "[*] development ROM restored" \
        || echo "[-] the development rebuild failed: see $OUT/dev-make.log" >&2
    return $status
}
trap restore_dev EXIT

# ---------------------------------------------------------------------------
step 2 "The game without the debug tools"
mkdir -p "$WORK/build-config"
printf '%s\n' "// scripts/package_client.sh $VERSION: a release build." "#define COOP_DEBUG_TOOLS 0" \
    > "$WORK/build-config/coop_build_config.h"
python3 scripts/avatar_build.py
(cd hg-engine && make clean_code > /dev/null \
    && C_INCLUDE_PATH="$WORK/build-config" make -j"$(nproc)" > "$OUT/release-make.log" 2>&1) \
    || die "the release build failed: see $OUT/release-make.log"
python3 scripts/extract_coop_sym.py
git diff --quiet -- coop_offsets.json || die "coop_offsets.json changed in the release build (the mailbox moved)"
for sym in Coop_DebugGiveTestMon Coop_DebugWildBattle gCoopDebugMonBuffer; do
    if arm-none-eabi-nm hg-engine/build/linked.o hg-engine/build/field_linked.o | grep -q " $sym$"; then
        die "the release ROM still has the debug tools ($sym)"
    fi
done
cp "$GAME" "$WORK/goldlink.nds"
PATCHED_SHA1="$(sha1sum "$WORK/goldlink.nds" | cut -d' ' -f1)"
echo "[*] the game: SHA-1 $PATCHED_SHA1, no debug tools"

# ---------------------------------------------------------------------------
step 3 "The patch"
cargo build --release --locked -p goldlink-dev
target/release/goldlink-dev --make-patch "$ROM" "$WORK/goldlink.nds" "$WORK/goldlink.bps"

# ---------------------------------------------------------------------------
step 4 "The program"
BUILD="GOLDLINK_BUILD=$VERSION-${COMMIT:0:9}${DIRTY:-}"
env "$BUILD" cargo build --release --locked -p goldlink --target x86_64-pc-windows-gnu
docker image inspect "$IMAGE" > /dev/null 2>&1 \
    || docker build -t "$IMAGE" -f scripts/release/client/Dockerfile scripts/release/client
mkdir -p "$CACHE/cargo-home"
docker run --rm --user "$(id -u):$(id -g)" \
    -v "$ROOT:/src" -v "$CACHE/cargo-home:/cargo-home" \
    -e CARGO_HOME=/cargo-home -e CARGO_TARGET_DIR=/src/target/bullseye -e "$BUILD" -w /src \
    "$IMAGE" cargo build --release --locked -p goldlink
LINUX_BIN="target/bullseye/release/goldlink"
WINDOWS_BIN="target/x86_64-pc-windows-gnu/release/goldlink.exe"
glibc="$(objdump -T "$LINUX_BIN" | grep -oP 'GLIBC_\K[0-9.]+' | sort -uV | tail -1)"
[[ "$(printf '%s\n%s\n' "$glibc" "$MAX_GLIBC" | sort -V | tail -1)" == "$MAX_GLIBC" ]] \
    || die "the Linux program needs glibc $glibc (at most $MAX_GLIBC)"
echo "[*] Linux program: glibc $glibc or newer"
# The release program carries no dev tools (DECISIONS D2).
for bin in "$LINUX_BIN" "$WINDOWS_BIN"; do
    if strings "$bin" | grep -qE 'COOP_BRIDGE_AUTOJOIN|COOP_BRIDGE_LEAVE_IN_BATTLE|GOLDLINK_DEV_PANEL|memorysavestate'; then
        die "$bin has dev tools in it"
    fi
done
odd="$(x86_64-w64-mingw32-objdump -p "$WINDOWS_BIN" | grep -oP 'DLL Name: \K.*' | sort -u \
    | grep -viE '^(api-ms-win-.*|kernel32|user32|gdi32|advapi32|ole32|oleaut32|shell32|ntdll|combase|bcrypt(primitives)?|ws2_32|wsock32|imm32|dwmapi|uxtheme|propsys|shlwapi|userenv|rpcrt4|opengl32|d3dcompiler_47|winmm|setupapi|hid|dxgi|d3d12|iphlpapi|secur32|crypt32|comctl32|comdlg32|msvcrt|ucrtbase|version|xinput.*)\.dll$' || true)"
[[ -z "$odd" ]] || die "the Windows program needs DLLs Windows doesn't have: $odd"

# The libraries' licenses: the program's dependencies only (not the dev tools').
cargo tree --locked -p goldlink -e normal --target all --prefix none --format '{p}' \
    | awk '{print $1" "$2}' | sed 's/ v/ /' | sort -u > "$WORK/deps.txt"
cargo metadata --format-version 1 --locked \
    | python3 -I -c '
import json, sys
deps = {tuple(l.split()) for l in open(sys.argv[1]) if l.strip()}
meta = json.load(sys.stdin)
meta["packages"] = [p for p in meta["packages"] if (p["name"], p["version"]) in deps and p["source"] is not None]
json.dump(meta, sys.stdout)' "$WORK/deps.txt" \
    | python3 -I scripts/release/crate_licenses.py > "$WORK/libraries.txt"

# ---------------------------------------------------------------------------
step 5 "The archives"
fill() { # template -> stdout
    sed -e "s|{{VERSION}}|$VERSION|g" -e "s|{{WIRE}}|$WIRE|g" -e "s|{{REPOSITORY}}|$REPOSITORY|g" \
        -e "s|{{MELONDS_VERSION}}|$MELONDS|g" -e "s|{{ROM_SHA1}}|$ROM_SHA1|g" "$1"
}
assemble() { # <os> <dir>
    local os="$1" dir="$2"
    mkdir -p "$dir/LICENSES" "$dir/build/avatars"
    if [[ $os == linux ]]; then cp "$LINUX_BIN" "$dir/GoldLink"; else cp "$WINDOWS_BIN" "$dir/GoldLink.exe"; fi
    cp "$WORK/goldlink.bps" coop_avatars.json "$dir/"
    python3 -I -c 'import json,sys; [print(a["preview"]) for a in json.load(open(sys.argv[1]))["avatars"] if "preview" in a]' \
        coop_avatars.json | while read -r preview; do cp "$preview" "$dir/$preview"; done
    cat > "$dir/release.json" <<JSON
{
  "version": "$NUMBER",
  "client": "unified",
  "wire_version": $WIRE,
  "rom_sha1": "$ROM_SHA1",
  "patched_sha1": "$PATCHED_SHA1",
  "patch": "goldlink.bps",
  "mailbox_offset": $MAILBOX_OFFSET,
  "mailbox_version": $MAILBOX_VERSION,
  "melonds": "$MELONDS",
  "repository": "$REPOSITORY",
  "commit": "$COMMIT${DIRTY:-}"
}
JSON
    fill scripts/release/client/README.txt > "$dir/README.txt"
    fill scripts/release/client/THIRD_PARTY.txt > "$dir/LICENSES/THIRD_PARTY.txt"
    cp crates/goldlink-core-sys/melonDS/LICENSE "$dir/LICENSES/COPYING"
    cp LICENSE "$dir/LICENSES/GoldLink-LICENSE.txt"
    cp NOTICE.md "$dir/LICENSES/NOTICE.md"
    cp CREDITS.md "$dir/CREDITS.md"
    cp "$WORK/libraries.txt" "$dir/LICENSES/libraries.txt"
    # The GPL's source offer: the exact source of this program, melonDS included.
    cat > "$dir/LICENSES/SOURCE.txt" <<TXT
GoldLink $VERSION is free software (GPL-3.0-or-later, COPYING). Its complete source, the
build scripts and melonDS ($MELONDS, a git submodule) included, is the commit

    https://github.com/$REPOSITORY/tree/$COMMIT

(git clone --recurse-submodules https://github.com/$REPOSITORY, then git checkout $COMMIT).
Build: cargo build --release -p goldlink (needs Rust, CMake and a C++17 compiler).
TXT
    if [[ $os == windows ]]; then # Windows' Notepad likes CRLF
        for f in README.txt LICENSES/THIRD_PARTY.txt LICENSES/SOURCE.txt; do sed -i 's/$/\r/' "$dir/$f"; done
    fi
}
NAME="GoldLink-$VERSION"
for os in linux windows; do
    assemble "$os" "$WORK/$os/$NAME"
done
tar czf "$OUT/$NAME-linux.tar.gz" -C "$WORK/linux" "$NAME"
python3 -I - "$WORK/windows" "$NAME" "$OUT/$NAME-windows.zip" <<'PY'
import os, sys, zipfile
base, name, out = sys.argv[1:4]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for dirpath, dirnames, files in os.walk(os.path.join(base, name)):
        dirnames.sort()
        for f in sorted(files):
            full = os.path.join(dirpath, f)
            z.write(full, os.path.relpath(full, base))
PY
(cd "$OUT" && sha256sum "$NAME-linux.tar.gz" "$NAME-windows.zip" > SHA256SUMS)
ls -l "$OUT"/*.tar.gz "$OUT"/*.zip

# ---------------------------------------------------------------------------
step 6 "Notes"
fill scripts/release/client/notes.md > "$OUT/notes.md"
if [[ -n "${NOTES:-}" ]]; then
    python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read().replace("{{CHANGES}}", open(sys.argv[2]).read().strip()); open(p,"w").write(s)' "$OUT/notes.md" "$NOTES"
else
    sed -i 's|{{CHANGES}}|(write the changes here)|' "$OUT/notes.md"
fi

# ---------------------------------------------------------------------------
step 7 "GitHub"
if [[ $PUBLISH == 1 ]]; then
    gh release create "$VERSION" -R "$REPOSITORY" --draft --target "$COMMIT" --title "GoldLink $VERSION" \
        --notes-file "$OUT/notes.md" "$OUT/$NAME-linux.tar.gz" "$OUT/$NAME-windows.zip" "$OUT/SHA256SUMS"
    echo "[*] draft release $VERSION created: review and publish it on GitHub"
else
    echo "[*] not published (--publish creates a draft GitHub release)"
fi

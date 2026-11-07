#!/usr/bin/env bash
# The player release (docs/RELEASE_PLAN.md §3.1, §6): GoldLink-vX.Y-windows.zip and
# GoldLink-vX.Y-linux.tar.gz in build/release/vX.Y/, built from HEAD with the debug tools off.
#
#   scripts/package_release.sh vX.Y [--publish]
#
#   --publish    also creates the GitHub release vX.Y as a draft (review it, then publish it on
#                GitHub). HEAD must be pushed.
#   NOTES=<file> the "Changes" section of the release notes (Markdown); a placeholder otherwise.
#   ALLOW_DIRTY=1  packages uncommitted changes (testing the script; never with --publish).
#   KEEP_RELEASE_BUILD=1  leaves hg-engine built without the debug tools (skips the rebuild of
#                the development ROM at the end).
#
# Needs: hg-engine/rom.nds (HeartGold USA), cargo with the x86_64-pc-windows-gnu target and
# mingw-w64-gcc, dotnet, docker (the Linux launcher is built on Debian 11 for an old glibc), gh.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || { echo "usage: $0 vX.Y [--publish]" >&2; exit 2; }
PUBLISH=0
[[ "${2:-}" == "--publish" ]] && PUBLISH=1
NUMBER="${VERSION#v}"

OUT="$ROOT/build/release/$VERSION"
WORK="$OUT/work"
CACHE="$ROOT/build/release-cache"
REPOSITORY="eloyball/pokemon-goldlink"
ROM="$ROOT/hg-engine/rom.nds"
GAME="$ROOT/hg-engine/test.nds"
BIZHAWK_VERSION="2.11.1"
BIZHAWK_LINUX="BizHawk-$BIZHAWK_VERSION-linux-x64.tar.gz"
BIZHAWK_LINUX_SHA256="38c9c12287e337a0a6923fd527767c853457d61d71e7d8ad1a772d64ce8bc93f"
BIZHAWK_WINDOWS="BizHawk-$BIZHAWK_VERSION-win-x64.zip"
BIZHAWK_WINDOWS_SHA256="dd7cbd5e205b09c5bce6afabc4f8aa525acc0bbeb9753b198a8db3fe6a5e5717"
LINUX_IMAGE="rust:1.98.0-bullseye" # Debian 11: glibc 2.31
MAX_GLIBC="2.31"

die() { echo "[-] $*" >&2; exit 1; }
step() { echo; echo "=== [$1/8] $2"; }

# ---------------------------------------------------------------------------
step 1 "Checks"
for tool in cargo dotnet docker gh x86_64-w64-mingw32-gcc arm-none-eabi-nm strings sha1sum sha256sum unzip; do
    command -v "$tool" > /dev/null || die "$tool is missing"
done
rustup target list --installed | grep -qx x86_64-pc-windows-gnu || die "rustup target add x86_64-pc-windows-gnu"
[[ -f "$ROM" ]] || die "$ROM is missing (your HeartGold USA ROM)"
COMMIT="$(git rev-parse HEAD)"
if ! git diff --quiet HEAD --; then
    [[ "${ALLOW_DIRTY:-0}" == 1 && $PUBLISH == 0 ]] || die "uncommitted changes: a release is built from a commit"
    echo "[!] ALLOW_DIRTY=1: built with uncommitted changes (a test package, never published)"
    DIRTY="-dirty"
fi
if git rev-parse -q --verify "refs/tags/$VERSION" > /dev/null; then
    [[ "$(git rev-parse "$VERSION^{commit}")" == "$COMMIT" ]] || die "tag $VERSION exists on another commit"
fi
if [[ $PUBLISH == 1 ]]; then
    git fetch -q origin
    git merge-base --is-ancestor "$COMMIT" origin/master || die "push HEAD first (--publish tags it on GitHub)"
    gh release view "$VERSION" -R "$REPOSITORY" > /dev/null 2>&1 && die "the GitHub release $VERSION exists already"
fi
python3 scripts/bizhawk_defaults.py --check
WIRE="$(grep -oP 'PROTOCOL_VERSION: u16 = \K[0-9]+' coop-relay/src/protocol.rs)"
BRIDGE_WIRE="$(grep -hoP 'ProtocolVersion\s*=\s*\K[0-9]+' coop-bridge/src/*.cs | head -1)"
[[ -n "$WIRE" && "$WIRE" == "$BRIDGE_WIRE" ]] || die "relay protocol $WIRE, bridge protocol $BRIDGE_WIRE"
ROM_SHA1="$(sha1sum "$ROM" | cut -d' ' -f1)"
echo "[*] $VERSION from ${COMMIT:0:9}; relay protocol $WIRE; ROM SHA-1 $ROM_SHA1"
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
printf '%s\n' "// scripts/package_release.sh $VERSION: a release build." "#define COOP_DEBUG_TOOLS 0" \
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
step 3 "The launchers"
cargo build --release --locked --manifest-path coop-launcher/Cargo.toml
cargo build --release --locked --manifest-path coop-launcher/Cargo.toml --target x86_64-pc-windows-gnu
mkdir -p "$CACHE/cargo-home"
docker run --rm --user "$(id -u):$(id -g)" \
    -v "$ROOT/coop-launcher:/src" -v "$CACHE/cargo-home:/cargo-home" \
    -e CARGO_HOME=/cargo-home -e CARGO_TARGET_DIR=/src/target/bullseye -w /src \
    "$LINUX_IMAGE" cargo build --release --locked
LINUX_LAUNCHER="coop-launcher/target/bullseye/release/GoldLink"
WINDOWS_LAUNCHER="coop-launcher/target/x86_64-pc-windows-gnu/release/GoldLink.exe"
glibc="$(objdump -T "$LINUX_LAUNCHER" | grep -oP 'GLIBC_\K[0-9.]+' | sort -uV | tail -1)"
[[ "$(printf '%s\n%s\n' "$glibc" "$MAX_GLIBC" | sort -V | tail -1)" == "$MAX_GLIBC" ]] \
    || die "the Linux launcher needs glibc $glibc (at most $MAX_GLIBC)"
echo "[*] Linux launcher: glibc $glibc or newer"

# The libraries' licenses, from the crates as cargo downloaded them.
cargo metadata --format-version 1 --locked --manifest-path coop-launcher/Cargo.toml \
        --filter-platform x86_64-unknown-linux-gnu --filter-platform x86_64-pc-windows-gnu \
    | python3 -I scripts/release/crate_licenses.py > "$WORK/launcher-libraries.txt"

# ---------------------------------------------------------------------------
step 4 "The patch"
"coop-launcher/target/release/GoldLink" --make-patch "$ROM" "$WORK/goldlink.nds" "$WORK/goldlink.bps"

# ---------------------------------------------------------------------------
step 5 "The bridge without the debug tools"
(cd coop-bridge/src && dotnet build -c Ship -nologo -v q) || die "the bridge build failed"
BRIDGE="coop-bridge/src/bin/Ship/CoopBridge.dll"
if strings -el "$BRIDGE" | grep -qE 'COOP_BRIDGE_AUTOJOIN|COOP_BRIDGE_LEAVE_IN_BATTLE|Show diagnostics'; then
    die "the release bridge still has the debug tools"
fi
echo "[*] $BRIDGE: no debug tools"

# ---------------------------------------------------------------------------
step 6 "BizHawk $BIZHAWK_VERSION"
fetch() { # <file> <sha256>
    [[ -f "$CACHE/$1" ]] || gh release download "$BIZHAWK_VERSION" -R TASEmulators/BizHawk -p "$1" -D "$CACHE"
    echo "$2  $CACHE/$1" | sha256sum -c --quiet || die "$1 doesn't match its pinned SHA-256"
}
fetch "$BIZHAWK_LINUX" "$BIZHAWK_LINUX_SHA256"
fetch "$BIZHAWK_WINDOWS" "$BIZHAWK_WINDOWS_SHA256"
[[ -f "$CACHE/BizHawk-LICENSE-$BIZHAWK_VERSION.txt" ]] \
    || gh api "repos/TASEmulators/BizHawk/contents/LICENSE?ref=$BIZHAWK_VERSION" -q .content \
        | base64 -d > "$CACHE/BizHawk-LICENSE-$BIZHAWK_VERSION.txt"

# ---------------------------------------------------------------------------
step 7 "The archives"
fill() { # template -> stdout
    sed -e "s|{{VERSION}}|$VERSION|g" -e "s|{{WIRE}}|$WIRE|g" -e "s|{{REPOSITORY}}|$REPOSITORY|g" \
        -e "s|{{BIZHAWK_VERSION}}|$BIZHAWK_VERSION|g" -e "s|{{ROM_SHA1}}|$ROM_SHA1|g" "$1"
}
assemble() { # <os> <dir>
    local os="$1" dir="$2"
    mkdir -p "$dir/BizHawk" "$dir/defaults" "$dir/LICENSES" "$dir/build/avatars"
    if [[ $os == linux ]]; then
        tar xzf "$CACHE/$BIZHAWK_LINUX" -C "$dir/BizHawk" --strip-components=1
        cp "$LINUX_LAUNCHER" "$dir/GoldLink"
    else
        unzip -q "$CACHE/$BIZHAWK_WINDOWS" -d "$dir/BizHawk"
        cp "$WINDOWS_LAUNCHER" "$dir/GoldLink.exe"
    fi
    [[ -f "$dir/BizHawk/EmuHawk.exe" ]] || die "BizHawk's archive has an unexpected layout"
    mkdir -p "$dir/BizHawk/ExternalTools"
    cp "$BRIDGE" "$dir/BizHawk/ExternalTools/CoopBridge.dll"
    cp "$WORK/goldlink.bps" coop_offsets.json coop_avatars.json "$dir/"
    cp coop-launcher/defaults/config.ini "$dir/defaults/config.ini"
    # Only the previews coop_avatars.json names.
    python3 -I -c 'import json,sys; [print(a["preview"]) for a in json.load(open(sys.argv[1]))["avatars"] if "preview" in a]' \
        coop_avatars.json | while read -r preview; do cp "$preview" "$dir/$preview"; done
    cat > "$dir/release.json" <<JSON
{
  "version": "$NUMBER",
  "wire_version": $WIRE,
  "rom_sha1": "$ROM_SHA1",
  "patched_sha1": "$PATCHED_SHA1",
  "patch": "goldlink.bps",
  "bizhawk_version": "$BIZHAWK_VERSION",
  "repository": "$REPOSITORY",
  "commit": "$COMMIT${DIRTY:-}"
}
JSON
    fill scripts/release/README.txt > "$dir/README.txt"
    fill scripts/release/THIRD_PARTY.txt > "$dir/LICENSES/THIRD_PARTY.txt"
    cp "$CACHE/BizHawk-LICENSE-$BIZHAWK_VERSION.txt" "$dir/LICENSES/BizHawk-LICENSE.txt"
    cp "$WORK/launcher-libraries.txt" "$dir/LICENSES/launcher-libraries.txt"
    if [[ $os == windows ]]; then # Windows' Notepad and Explorer like CRLF
        for f in README.txt LICENSES/THIRD_PARTY.txt; do sed -i 's/$/\r/' "$dir/$f"; done
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
fill scripts/release/notes.md > "$OUT/notes.md"
if [[ -n "${NOTES:-}" ]]; then
    python3 -I -c 'import sys; p=sys.argv[1]; s=open(p).read().replace("{{CHANGES}}", open(sys.argv[2]).read().strip()); open(p,"w").write(s)' "$OUT/notes.md" "$NOTES"
else
    sed -i 's|{{CHANGES}}|(write the changes here)|' "$OUT/notes.md"
fi
(cd "$OUT" && sha256sum "$NAME-linux.tar.gz" "$NAME-windows.zip" > SHA256SUMS)
ls -l "$OUT"/*.tar.gz "$OUT"/*.zip

# ---------------------------------------------------------------------------
step 8 "GitHub"
if [[ $PUBLISH == 1 ]]; then
    gh release create "$VERSION" -R "$REPOSITORY" --draft --target "$COMMIT" --title "GoldLink $VERSION" \
        --notes-file "$OUT/notes.md" "$OUT/$NAME-linux.tar.gz" "$OUT/$NAME-windows.zip" "$OUT/SHA256SUMS"
    echo "[*] draft release $VERSION created: review and publish it on GitHub"
else
    echo "[*] not published (--publish creates a draft GitHub release)"
fi

#!/usr/bin/env bash
# The relay release (docs/RELEASE_PLAN.md §3.2, §6): coop-relay-vX.Y-linux-x86_64.tar.gz,
# -linux-aarch64.tar.gz and -windows.zip in build/relay-release/vX.Y/, built from HEAD. Needs no
# ROM: GitHub Actions runs it on a relay-v* tag (.github/workflows/relay-release.yml).
#
#   scripts/package_relay.sh relay-vX.Y [--publish]
#
#   relay-vX.Y must match coop-relay/Cargo.toml's version (relay-v0.2 for 0.2.0, or relay-v0.2.0).
#   --publish    also creates the GitHub release relay-vX.Y as a draft (review it, then publish
#                it on GitHub). HEAD must be pushed.
#   NOTES=<file> the "Changes" section of the release notes (Markdown).
#   ALLOW_DIRTY=1  packages uncommitted changes (testing the script; never with --publish).
#
# Needs: cargo with the x86_64-unknown-linux-musl, aarch64-unknown-linux-musl and
# x86_64-pc-windows-gnu targets, mingw-w64-gcc, python3, gh (--publish).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAG="${1:-}"
[[ "$TAG" =~ ^relay-v([0-9]+\.[0-9]+(\.[0-9]+)?)$ ]] || { echo "usage: $0 relay-vX.Y [--publish]" >&2; exit 2; }
NUMBER="${BASH_REMATCH[1]}"
VERSION="v$NUMBER"
PUBLISH=0
[[ "${2:-}" == "--publish" ]] && PUBLISH=1
REPOSITORY="eloyball/pokemon-goldlink"
OUT="$ROOT/build/relay-release/$VERSION"
MANIFEST="coop-relay/Cargo.toml"
TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu)

die() { echo "[-] $*" >&2; exit 1; }

# --- Checks
CRATE="$(grep -m1 -oP '^version = "\K[^"]+' "$MANIFEST")"
[[ "$CRATE" == "$NUMBER" || "$CRATE" == "$NUMBER.0" ]] || die "$TAG doesn't match $MANIFEST's version $CRATE"
WIRE="$(grep -oP 'PROTOCOL_VERSION: u16 = \K[0-9]+' coop-relay/src/protocol.rs)"
for t in "${TARGETS[@]}"; do
    rustup target list --installed | grep -qx "$t" || die "rustup target add $t"
done
command -v x86_64-w64-mingw32-gcc > /dev/null || die "x86_64-w64-mingw32-gcc is missing (mingw-w64-gcc)"
COMMIT="$(git rev-parse HEAD)"
# crates/goldlink-proto is the relay's wire protocol (compiled from coop-relay/src/protocol.rs);
# the root Cargo.toml is the workspace it inherits its edition from.
if ! git diff --quiet HEAD -- coop-relay crates/goldlink-proto Cargo.toml scripts/package_relay.sh LICENSE NOTICE.md; then
    [[ "${ALLOW_DIRTY:-0}" == 1 && $PUBLISH == 0 ]] || die "uncommitted changes: a release is built from a commit"
    echo "[!] ALLOW_DIRTY=1: built with uncommitted changes (a test package, never published)"
fi
if git rev-parse -q --verify "refs/tags/$TAG" > /dev/null; then
    [[ "$(git rev-parse "$TAG^{commit}")" == "$COMMIT" ]] || die "tag $TAG exists on another commit"
fi
if [[ $PUBLISH == 1 ]]; then
    gh release view "$TAG" -R "$REPOSITORY" > /dev/null 2>&1 && die "the GitHub release $TAG exists already"
fi
echo "[*] coop-relay $CRATE ($TAG) from ${COMMIT:0:9}: relay protocol $WIRE"

# --- Build (static on Linux; the aarch64 one links with the toolchain's own lld)
cargo test --locked --manifest-path "$MANIFEST" -q
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
for t in "${TARGETS[@]}"; do
    cargo build --release --locked --manifest-path "$MANIFEST" --target "$t"
done

# --- Archives
rm -rf "$OUT"
mkdir -p "$OUT/work"
fill() {
    sed -e "s|{{VERSION}}|$VERSION|g" -e "s|{{WIRE}}|$WIRE|g" -e "s|{{REPOSITORY}}|$REPOSITORY|g" "$1"
}
NAME="coop-relay-$VERSION"
pack() { # <target> <archive suffix>
    local target="$1" suffix="$2" dir="$OUT/work/$2/$NAME" exe="coop-relay"
    [[ $target == *windows* ]] && exe="coop-relay.exe"
    mkdir -p "$dir"
    cp "coop-relay/target/$target/release/$exe" "$dir/"
    fill coop-relay/release/README.md > "$dir/README.md"
    cp LICENSE "$dir/LICENSE"
    cp NOTICE.md "$dir/NOTICE.md"
    cargo metadata --format-version 1 --locked --manifest-path "$MANIFEST" --filter-platform "$target" \
        | python3 -I scripts/release/crate_licenses.py > "$dir/THIRD_PARTY_LICENSES.txt"
    if [[ $target == *linux* ]]; then
        cp coop-relay/release/coop-relay.service coop-relay/release/Dockerfile "$dir/"
        tar czf "$OUT/$NAME-$suffix.tar.gz" -C "$OUT/work/$suffix" "$NAME"
    else
        for f in README.md LICENSE NOTICE.md THIRD_PARTY_LICENSES.txt; do sed -i 's/$/\r/' "$dir/$f"; done
        python3 -I - "$OUT/work/$suffix" "$NAME" "$OUT/$NAME-$suffix.zip" <<'PY'
import os, sys, zipfile
base, name, out = sys.argv[1:4]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for dirpath, dirnames, files in os.walk(os.path.join(base, name)):
        dirnames.sort()
        for f in sorted(files):
            full = os.path.join(dirpath, f)
            z.write(full, os.path.relpath(full, base))
PY
    fi
}
pack x86_64-unknown-linux-musl linux-x86_64
pack aarch64-unknown-linux-musl linux-aarch64
pack x86_64-pc-windows-gnu windows
(cd "$OUT" && sha256sum "$NAME"-* > SHA256SUMS)

cat > "$OUT/notes.md" <<EOF
The GoldLink relay server, **relay protocol $WIRE**: for the GoldLink player releases that need
protocol $WIRE (their release notes say). A relay for protocol $WIRE refuses players of releases for
another protocol (they get a message saying which side is behind): update the relay and every
player together.

| File | For |
| :--- | :--- |
| \`$NAME-linux-x86_64.tar.gz\` | Linux, x86-64 (a static binary: any distribution) |
| \`$NAME-linux-aarch64.tar.gz\` | Linux, ARM64 (Raspberry Pi, Oracle Cloud Ampere...) |
| \`$NAME-windows.zip\` | Windows, 64-bit |

Each archive's \`README.md\` covers running it, opening UDP ports 7777 and 7778 (the encrypted
port), systemd, Docker, the options and backing up the campaigns and the certificate. GoldLink is free: CC BY-NC-SA 4.0, on hg-engine's terms (no
money involved, donations included).

## Changes

$(if [[ -n "${NOTES:-}" ]]; then cat "$NOTES"; else echo "(write the changes here)"; fi)
EOF
ls -l "$OUT"/coop-relay-*

if [[ $PUBLISH == 1 ]]; then
    gh release create "$TAG" -R "$REPOSITORY" --draft --target "$COMMIT" --title "coop-relay $VERSION" \
        --notes-file "$OUT/notes.md" "$OUT/$NAME"-* "$OUT/SHA256SUMS"
    echo "[*] draft release $TAG created: review and publish it on GitHub"
else
    echo "[*] not published (--publish creates a draft GitHub release)"
fi

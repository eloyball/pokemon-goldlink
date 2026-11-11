#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "============================================================"
echo " Building Pokémon HeartGold 6-Player Co-op Stack"
echo "============================================================"

echo "[1/3] Testing & building coop-relay (Rust)..."
cd "$ROOT_DIR/coop-relay"
cargo test
cargo build --release

echo "[2/3] Cutting the avatar sheets, then compiling hg-engine ROM (C / ARMIPS)..."
cd "$ROOT_DIR"
python3 ./scripts/avatar_build.py   # docs/CUSTOM_SPRITES_PLAN.md: writes only what changed
cd "$ROOT_DIR/hg-engine"
make -j$(nproc)

echo "[3/3] Extracting & validating gCoopCommBlock symbol offset..."
cd "$ROOT_DIR"
python3 ./scripts/extract_coop_sym.py

# The game itself: cargo build --release -p goldlink (scripts/package_client.sh for a release).

echo "============================================================"
echo " Build Complete Successfully!"
echo "============================================================"
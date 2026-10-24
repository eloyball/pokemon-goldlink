#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "============================================================"
echo " Building Pokémon HeartGold 6-Player Co-op Stack"
echo "============================================================"

echo "[1/4] Testing & building coop-relay (Rust)..."
cd "$ROOT_DIR/coop-relay"
cargo test
cargo build --release

echo "[2/4] Compiling hg-engine ROM (C / ARMIPS)..."
cd "$ROOT_DIR/hg-engine"
make -j$(nproc)

echo "[3/4] Extracting & validating gCoopCommBlock symbol offset..."
cd "$ROOT_DIR"
python3 ./scripts/extract_coop_sym.py

echo "[4/4] Building CoopBridge.dll (C#) and installing to BizHawk/ExternalTools..."
cd "$ROOT_DIR/coop-bridge/src"
dotnet build -c Release
mkdir -p "$ROOT_DIR/BizHawk/ExternalTools"
cp bin/Release/CoopBridge.dll "$ROOT_DIR/BizHawk/ExternalTools/CoopBridge.dll"

echo "============================================================"
echo " Build Complete Successfully!"
echo "============================================================"
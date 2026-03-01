#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Game install path (deploy target)
GAME_DIR="${ErenshorGamePath:-/mnt/d/SteamLibrary/steamapps/common/Erenshor}"
PLUGIN_DIR="$GAME_DIR/BepInEx/plugins/ErenshorLLMDialog"

# ONNX Runtime is loaded dynamically; point to the shipped copy for Linux index builds.
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$SCRIPT_DIR/data/lib/libonnxruntime.so}"

WIN_TARGET="x86_64-pc-windows-gnu"
WIN_EXE="target/$WIN_TARGET/release/erenshor-llm.exe"

# Isolate native Linux build into a separate target directory to prevent
# cargo cache contamination of the Windows cross-compile.
NATIVE_TARGET_DIR="target-native"
NATIVE_EXE="$NATIVE_TARGET_DIR/release/erenshor-llm"

SHIMMY_VERSION="v1.9.0"
SHIMMY_URL="https://github.com/Michael-A-Kuykendall/shimmy/releases/download/${SHIMMY_VERSION}/shimmy-windows-x86_64.exe"
SHIMMY_EXE="shimmy.exe"

if [ ! -f "$SHIMMY_EXE" ]; then
    echo "=== Downloading shimmy ${SHIMMY_VERSION} ==="
    curl -L -o "$SHIMMY_EXE" "$SHIMMY_URL"
    echo "Downloaded: $(ls -lh "$SHIMMY_EXE" | awk '{print $5}')"
else
    echo "=== shimmy already downloaded ==="
fi

echo "=== Building native binary (for index generation) ==="
cargo build --release --target-dir "$NATIVE_TARGET_DIR"

echo "=== Building Windows binary (cross-compile) ==="
# No cmake, no Vulkan SDK, no C++ compiler needed.
# Local LLM inference is handled by shimmy (separate pre-built binary).
cargo build --release --target "$WIN_TARGET"

echo "=== Ensuring dist directory exists ==="
mkdir -p data/dist

echo "=== Building lore index ==="
./"$NATIVE_EXE" --data-dir data build-index --input lore --output dist/lore.ruvector

echo "=== Building response templates ==="
./"$NATIVE_EXE" --data-dir data build-responses --input templates --output dist/responses.ruvector

echo "=== Deploying to game directory ==="
if [ -d "$PLUGIN_DIR" ]; then
    cp "$WIN_EXE" "$PLUGIN_DIR/erenshor-llm.exe"
    cp "$SHIMMY_EXE" "$PLUGIN_DIR/shimmy.exe"
    cp data/dist/*.ruvector "$PLUGIN_DIR/dist/"
    sync
    echo "Deployed to $PLUGIN_DIR"
else
    echo "WARNING: Plugin dir not found at $PLUGIN_DIR"
    echo "Set ErenshorGamePath env var or copy manually."
fi

echo ""
echo "=== Build complete ==="
echo "Windows binary:"
ls -lh "$WIN_EXE" 2>/dev/null || echo "  (not found)"
echo "Shimmy binary:"
ls -lh "$SHIMMY_EXE" 2>/dev/null || echo "  (not found)"
echo ""
echo "Shipped artifacts (data/dist/):"
ls -lh data/dist/*.ruvector 2>/dev/null || echo "  (no .ruvector files found)"
echo ""
echo "Note: personality.ruvector is NOT pre-built."
echo "It is rebuilt at sidecar startup from data/personalities/*.md"

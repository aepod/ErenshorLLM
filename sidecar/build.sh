#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Game install path (deploy target)
GAME_DIR="${ErenshorGamePath:-/mnt/d/SteamLibrary/steamapps/common/Erenshor}"
PLUGIN_DIR="$GAME_DIR/BepInEx/plugins/ErenshorLLMDialog"

# ONNX Runtime is loaded dynamically; point to the shipped copy for Linux index builds.
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$SCRIPT_DIR/data/lib/libonnxruntime.so}"

# llama-cpp-sys-2 uses bindgen which needs libclang.
export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib/llvm-14/lib}"

WIN_TARGET="x86_64-pc-windows-gnu"
WIN_EXE="target/$WIN_TARGET/release/erenshor-llm.exe"

# Isolate native Linux build into a separate target directory to prevent
# cargo/cmake cache contamination of the Windows Vulkan cross-compile.
NATIVE_TARGET_DIR="target-native"
NATIVE_EXE="$NATIVE_TARGET_DIR/release/erenshor-llm"

echo "=== Building native binary (for index generation) ==="
cargo build --release --target-dir "$NATIVE_TARGET_DIR"

echo "=== Building Windows binary (cross-compile with Vulkan) ==="
# Clear stale llama cmake cache to ensure GGML_VULKAN=ON is picked up.
# Cargo doesn't detect env var changes for cmake rebuilds, so a stale
# cache with GGML_VULKAN=OFF would persist silently.
rm -rf "target/$WIN_TARGET/release/build/llama-cpp-sys-2-"*/
# Set CMAKE_TOOLCHAIN_FILE and VULKAN_SDK only for cross-compile.
# These MUST NOT be set globally (e.g. in .cargo/config.toml [env]) because
# they would contaminate the native Linux build with mingw settings.
CMAKE_TOOLCHAIN_FILE="$SCRIPT_DIR/mingw-posix-toolchain.cmake" \
VULKAN_SDK="$SCRIPT_DIR/vulkan-sdk-mingw" \
cargo build --release --target "$WIN_TARGET" --features vulkan

echo "=== Ensuring dist directory exists ==="
mkdir -p data/dist

echo "=== Building lore index ==="
./"$NATIVE_EXE" --data-dir data build-index --input lore --output dist/lore.ruvector

echo "=== Building response templates ==="
./"$NATIVE_EXE" --data-dir data build-responses --input templates --output dist/responses.ruvector

echo "=== Deploying to game directory ==="
if [ -d "$PLUGIN_DIR" ]; then
    cp "$WIN_EXE" "$PLUGIN_DIR/erenshor-llm.exe"
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
echo ""
echo "Shipped artifacts (data/dist/):"
ls -lh data/dist/*.ruvector 2>/dev/null || echo "  (no .ruvector files found)"
echo ""
echo "Note: personality.ruvector is NOT pre-built."
echo "It is rebuilt at sidecar startup from data/personalities/*.json"

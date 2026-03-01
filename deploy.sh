#!/usr/bin/env bash
# deploy.sh -- Deploy ErenshorLLMDialog mod to a target directory.
#
# Usage:
#   ./deploy.sh [TARGET_DIR]
#
# If TARGET_DIR is omitted, defaults to:
#   $ErenshorGamePath/BepInEx/plugins/ErenshorLLMDialog
#
# Prerequisites:
#   - Rust sidecar built:  cargo build --release --target x86_64-pc-windows-gnu
#     (run from sidecar/ directory)
#   - C# mod built:  dotnet build ErenshorLLMDialog.csproj ...
#     (run from mod/ directory)
#
# What gets deployed:
#   TARGET_DIR/
#   +-- ErenshorLLMDialog.dll        # C# BepInEx mod
#   +-- erenshor-llm.exe             # Rust sidecar (RAG + routing)
#   +-- shimmy.exe                   # Local LLM inference server (GPU/CPU)
#   +-- onnxruntime.dll              # ONNX Runtime (if present)
#   +-- data/
#       +-- erenshor-llm.toml        # Config (only if not already present)
#       +-- dist/                    # Pre-built vector indexes
#       |   +-- lore.json, lore.ruvector, responses.json, responses.ruvector
#       +-- models/                  # Embedding model (ONNX)
#       |   +-- all-minilm-l6-v2.onnx, tokenizer.json
#       +-- personalities/           # Character personality files (.md)
#       +-- lore/                    # Source lore markdown files
#       +-- templates/               # Source response template files

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SIDECAR_DIR="$SCRIPT_DIR/sidecar"
MOD_DIR="$SCRIPT_DIR/mod"

# Resolve target directory
if [ $# -ge 1 ]; then
    TARGET="$1"
else
    if [ -z "${ErenshorGamePath:-}" ]; then
        echo "Error: No target directory provided and \$ErenshorGamePath is not set."
        echo "Usage: $0 [TARGET_DIR]"
        echo "  or:  export ErenshorGamePath=/path/to/Erenshor && $0"
        exit 1
    fi
    TARGET="$ErenshorGamePath/BepInEx/plugins/ErenshorLLMDialog"
fi

echo "Deploying ErenshorLLMDialog to: $TARGET"

# --- Locate build artifacts ---

# Sidecar binary
SIDECAR_EXE="$SIDECAR_DIR/target/x86_64-pc-windows-gnu/release/erenshor-llm.exe"
if [ ! -f "$SIDECAR_EXE" ]; then
    echo "Error: Sidecar binary not found at $SIDECAR_EXE"
    echo "Build it first: cd sidecar && cargo build --release --target x86_64-pc-windows-gnu"
    exit 1
fi

# C# mod DLL
MOD_DLL="$MOD_DIR/bin/Debug/netstandard2.1/ErenshorLLMDialog.dll"
if [ ! -f "$MOD_DLL" ]; then
    echo "Error: Mod DLL not found at $MOD_DLL"
    echo "Build it first: cd mod && dotnet build ErenshorLLMDialog.csproj ..."
    exit 1
fi

# Source data directory
DATA_SRC="$SIDECAR_DIR/data"
if [ ! -d "$DATA_SRC" ]; then
    echo "Error: Sidecar data directory not found at $DATA_SRC"
    exit 1
fi

# --- Create target directories ---

mkdir -p "$TARGET"
mkdir -p "$TARGET/data/dist"
mkdir -p "$TARGET/data/models"
mkdir -p "$TARGET/data/personalities"
mkdir -p "$TARGET/data/lore"
mkdir -p "$TARGET/data/templates"

# --- Deploy binaries ---

echo "  Deploying erenshor-llm.exe..."
cp "$SIDECAR_EXE" "$TARGET/erenshor-llm.exe"

echo "  Deploying ErenshorLLMDialog.dll..."
cp "$MOD_DLL" "$TARGET/ErenshorLLMDialog.dll"

# Shimmy local inference server (for local/hybrid LLM mode)
SHIMMY_EXE="$SIDECAR_DIR/shimmy.exe"
if [ -f "$SHIMMY_EXE" ]; then
    echo "  Deploying shimmy.exe..."
    cp "$SHIMMY_EXE" "$TARGET/shimmy.exe"
elif [ -f "$TARGET/shimmy.exe" ]; then
    echo "  shimmy.exe already in target (keeping)"
else
    echo "  Warning: shimmy.exe not found. Local LLM inference will not be available."
    echo "           Run: cd sidecar && curl -L -o shimmy.exe https://github.com/Michael-A-Kuykendall/shimmy/releases/download/v1.9.0/shimmy-windows-x86_64.exe"
fi

# ONNX Runtime DLL (optional, may be alongside the sidecar or in lib/)
ONNX_DLL=""
for candidate in "$SIDECAR_DIR/onnxruntime.dll" "$SIDECAR_DIR/data/lib/onnxruntime.dll" "$TARGET/onnxruntime.dll"; do
    if [ -f "$candidate" ]; then
        ONNX_DLL="$candidate"
        break
    fi
done
if [ -n "$ONNX_DLL" ] && [ "$ONNX_DLL" != "$TARGET/onnxruntime.dll" ]; then
    echo "  Deploying onnxruntime.dll..."
    cp "$ONNX_DLL" "$TARGET/onnxruntime.dll"
elif [ -f "$TARGET/onnxruntime.dll" ]; then
    echo "  onnxruntime.dll already in target (keeping)"
else
    echo "  Warning: onnxruntime.dll not found. ONNX embedding may not work."
fi

# Clean up legacy MinGW runtime DLLs (no longer needed since shimmy handles native code)
for legacy_dll in libstdc++-6.dll libgcc_s_seh-1.dll libgomp-1.dll libwinpthread-1.dll; do
    if [ -f "$TARGET/$legacy_dll" ]; then
        rm "$TARGET/$legacy_dll"
        echo "  Removed legacy $legacy_dll"
    fi
done

# --- Deploy config (don't overwrite user edits) ---

CONFIG_SRC="$SIDECAR_DIR/erenshor-llm.toml"
CONFIG_DST="$TARGET/data/erenshor-llm.toml"
if [ ! -f "$CONFIG_DST" ]; then
    echo "  Deploying erenshor-llm.toml (fresh install)..."
    cp "$CONFIG_SRC" "$CONFIG_DST"
else
    echo "  Config exists, deploying .default alongside..."
    cp "$CONFIG_SRC" "$CONFIG_DST.default"
fi

# --- Deploy data files ---

# Vector indexes (pre-built)
echo "  Deploying vector indexes..."
for f in "$DATA_SRC/dist/"*; do
    [ -f "$f" ] && cp "$f" "$TARGET/data/dist/"
done

# Embedding model
echo "  Deploying embedding model..."
for f in "$DATA_SRC/models/"*; do
    [ -f "$f" ] && cp "$f" "$TARGET/data/models/"
done

# Personality files
echo "  Deploying personality files..."
cp "$DATA_SRC/personalities/"*.md "$TARGET/data/personalities/" 2>/dev/null || true
cp "$DATA_SRC/personalities/"*.json "$TARGET/data/personalities/" 2>/dev/null || true
PCOUNT=$(ls "$TARGET/data/personalities/" 2>/dev/null | wc -l)
echo "    $PCOUNT personality files"

# Lore source files
# Skipped as it is now embedded in the sidecar and not needed by the mod directly. 
# The lore index is pre-built and deployed.
#echo "  Deploying lore files..."
#cp -r "$DATA_SRC/lore/"* "$TARGET/data/lore/" 2>/dev/null || true

# Template source files
echo "  Deploying template files..."
cp -r "$DATA_SRC/templates/"* "$TARGET/data/templates/" 2>/dev/null || true

# --- Summary ---

sync 2>/dev/null || true

echo ""
echo "=== Deployment complete ==="
echo "  Target:        $TARGET"
echo "  Sidecar:       $(ls -lh "$TARGET/erenshor-llm.exe" | awk '{print $5}')"
echo "  Shimmy:        $(ls -lh "$TARGET/shimmy.exe" 2>/dev/null | awk '{print $5}' || echo 'not deployed')"
echo "  Mod DLL:       $(ls -lh "$TARGET/ErenshorLLMDialog.dll" | awk '{print $5}')"
echo "  Personalities: $PCOUNT files"
echo "  Indexes:       $(ls "$TARGET/data/dist/" | wc -l) files"
echo "  Config:        $CONFIG_DST"
echo ""
echo "To test: launch Erenshor with BepInEx. Check BepInEx/LogOutput.log for startup messages."

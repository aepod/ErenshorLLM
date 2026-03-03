#!/usr/bin/env bash
# deploy.sh -- Deploy or package ErenshorLLMDialog mod.
#
# Usage:
#   ./deploy.sh [TARGET_DIR]       Deploy to game directory
#   ./deploy.sh --package          Create distributable zip (no deploy)
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
# What gets packaged/deployed:
#   ErenshorLLMDialog/
#   +-- README.md                    # Mod description and usage guide
#   +-- ErenshorLLMDialog.dll        # C# BepInEx mod
#   +-- erenshor-llm.exe             # Rust sidecar (RAG + routing)
#   +-- shimmy.exe                   # Local LLM inference server (GPU/CPU)
#   +-- onnxruntime.dll              # ONNX Runtime (if present)
#   +-- data/
#       +-- erenshor-llm.toml        # Config
#       +-- grounding.json           # Entity validation data
#       +-- dist/                    # Pre-built vector indexes
#       |   +-- lore.ruvector, responses.ruvector, personality.ruvector
#       +-- models/                  # Embedding model (ONNX) + GGUF models
#       |   +-- all-minilm-l6-v2.onnx, tokenizer.json
#       +-- personalities/           # Character personality files (.json, .md)
#       +-- templates/               # Source response template files

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SIDECAR_DIR="$SCRIPT_DIR/sidecar"
MOD_DIR="$SCRIPT_DIR/mod"
DOCS_DIR="$SCRIPT_DIR/docs"
MOD_VERSION="0.3-2"

# Parse flags
PACKAGE_MODE=false
POSITIONAL_ARGS=()
for arg in "$@"; do
    case "$arg" in
        --package) PACKAGE_MODE=true ;;
        *) POSITIONAL_ARGS+=("$arg") ;;
    esac
done

# --- Locate build artifacts (needed for both modes) ---

SIDECAR_EXE="$SIDECAR_DIR/target-cross/x86_64-pc-windows-gnu/release/erenshor-llm.exe"
if [ ! -f "$SIDECAR_EXE" ]; then
    SIDECAR_EXE="$SIDECAR_DIR/target/x86_64-pc-windows-gnu/release/erenshor-llm.exe"
fi
if [ ! -f "$SIDECAR_EXE" ]; then
    echo "Error: Sidecar binary not found at $SIDECAR_EXE"
    echo "Build it first: cd sidecar && cargo build --release --target x86_64-pc-windows-gnu"
    exit 1
fi

MOD_DLL="$MOD_DIR/bin/Debug/netstandard2.1/ErenshorLLMDialog.dll"
if [ ! -f "$MOD_DLL" ]; then
    echo "Error: Mod DLL not found at $MOD_DLL"
    echo "Build it first: cd mod && dotnet build ErenshorLLMDialog.csproj ..."
    exit 1
fi

DATA_SRC="$SIDECAR_DIR/data"
if [ ! -d "$DATA_SRC" ]; then
    echo "Error: Sidecar data directory not found at $DATA_SRC"
    exit 1
fi

# ═══════════════════════════════════════════════════════════════
# Package mode: assemble into staging dir, zip, done.
# ═══════════════════════════════════════════════════════════════
if $PACKAGE_MODE; then
    ZIP_DIR="$SCRIPT_DIR/dist"
    STAGING="$ZIP_DIR/staging/ErenshorLLMDialog"
    ZIP_NAME="ErenshorLLMDialog-v${MOD_VERSION}.zip"
    ZIP_PATH="$ZIP_DIR/$ZIP_NAME"

    echo "Packaging ErenshorLLMDialog v${MOD_VERSION}..."

    rm -rf "$ZIP_DIR/staging"
    mkdir -p "$STAGING/data/dist"
    mkdir -p "$STAGING/data/models"
    mkdir -p "$STAGING/data/personalities"
    mkdir -p "$STAGING/data/templates"

    # Binaries
    echo "  erenshor-llm.exe"
    cp "$SIDECAR_EXE" "$STAGING/erenshor-llm.exe"

    echo "  ErenshorLLMDialog.dll"
    cp "$MOD_DLL" "$STAGING/ErenshorLLMDialog.dll"

    SHIMMY_EXE="$SIDECAR_DIR/shimmy.exe"
    if [ -f "$SHIMMY_EXE" ]; then
        echo "  shimmy.exe"
        cp "$SHIMMY_EXE" "$STAGING/shimmy.exe"
    else
        echo "  Warning: shimmy.exe not found -- will not be included in package"
    fi

    # ONNX Runtime
    ONNX_DLL=""
    for candidate in "$SIDECAR_DIR/onnxruntime.dll" "$SIDECAR_DIR/data/onnxruntime.dll" "$SIDECAR_DIR/data/lib/onnxruntime.dll"; do
        if [ -f "$candidate" ]; then
            ONNX_DLL="$candidate"
            break
        fi
    done
    if [ -n "$ONNX_DLL" ]; then
        echo "  onnxruntime.dll"
        cp "$ONNX_DLL" "$STAGING/onnxruntime.dll"
    else
        echo "  Warning: onnxruntime.dll not found -- will not be included in package"
    fi

    # Config (always fresh copy for packages)
    echo "  erenshor-llm.toml"
    cp "$SIDECAR_DIR/erenshor-llm.toml" "$STAGING/data/erenshor-llm.toml"

    # Vector indexes
    echo "  Vector indexes..."
    for f in "$DATA_SRC/dist/"*; do
        [ -f "$f" ] && cp "$f" "$STAGING/data/dist/"
    done

    # Embedding model
    echo "  Embedding model..."
    for f in "$DATA_SRC/models/"*; do
        [ -f "$f" ] && cp "$f" "$STAGING/data/models/"
    done

    # Personality files
    cp "$DATA_SRC/personalities/"*.md "$STAGING/data/personalities/" 2>/dev/null || true
    cp "$DATA_SRC/personalities/"*.json "$STAGING/data/personalities/" 2>/dev/null || true
    PCOUNT=$(ls "$STAGING/data/personalities/" 2>/dev/null | wc -l)
    echo "  $PCOUNT personality files"

    # Templates
    echo "  Template files..."
    cp -r "$DATA_SRC/templates/"* "$STAGING/data/templates/" 2>/dev/null || true

    # Grounding data
    if [ -f "$DATA_SRC/grounding.json" ]; then
        echo "  grounding.json"
        cp "$DATA_SRC/grounding.json" "$STAGING/data/grounding.json"
    fi

    # README
    MOD_README="$DOCS_DIR/MOD-README.md"
    if [ -f "$MOD_README" ]; then
        echo "  README.md"
        cp "$MOD_README" "$STAGING/README.md"
    fi

    # BepInEx loads DLLs from the root plugins/ directory.
    # Place a copy alongside the subfolder so extraction works correctly.
    echo "  ErenshorLLMDialog.dll (plugins/ root)"
    cp "$MOD_DLL" "$ZIP_DIR/staging/ErenshorLLMDialog.dll"

    # Create zip
    rm -f "$ZIP_PATH"
    echo ""
    echo "Creating $ZIP_NAME..."
    (cd "$ZIP_DIR/staging" && zip -r "$ZIP_PATH" ErenshorLLMDialog.dll ErenshorLLMDialog/ -x '*.old' '*.new')

    # Clean up staging
    rm -rf "$ZIP_DIR/staging"

    echo ""
    echo "=== Package created ==="
    echo "  $(ls -lh "$ZIP_PATH" | awk '{print $5, $NF}')"
    echo ""
    echo "Users extract into: Erenshor/BepInEx/plugins/"
    exit 0
fi

# ═══════════════════════════════════════════════════════════════
# Deploy mode: copy to game directory.
# ═══════════════════════════════════════════════════════════════

if [ ${#POSITIONAL_ARGS[@]} -ge 1 ]; then
    TARGET="${POSITIONAL_ARGS[0]}"
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

# BepInEx loads plugins from the root plugins/ directory, not from subfolders.
# Copy the DLL there too so BepInEx picks up the latest version.
PLUGINS_ROOT="$(dirname "$TARGET")"
if [ -d "$PLUGINS_ROOT" ] && [ "$(basename "$PLUGINS_ROOT")" = "plugins" ]; then
    echo "  Deploying ErenshorLLMDialog.dll to plugins/ root..."
    cp "$MOD_DLL" "$PLUGINS_ROOT/ErenshorLLMDialog.dll"
fi

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
for candidate in "$SIDECAR_DIR/onnxruntime.dll" "$SIDECAR_DIR/data/onnxruntime.dll" "$SIDECAR_DIR/data/lib/onnxruntime.dll" "$TARGET/onnxruntime.dll"; do
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

# Grounding data (GEPA entity lists for LLM hallucination prevention)
if [ -f "$DATA_SRC/grounding.json" ]; then
    echo "  Deploying grounding.json..."
    cp "$DATA_SRC/grounding.json" "$TARGET/data/grounding.json"
fi

# --- Deploy MOD README ---

MOD_README="$DOCS_DIR/MOD-README.md"
if [ -f "$MOD_README" ]; then
    echo "  Deploying README.md..."
    cp "$MOD_README" "$TARGET/README.md"
fi

# --- Clean up stale directories from previous builds ---

for stale in "$TARGET/dist" "$TARGET/databuild-index" "$TARGET/erenshor-llm.exe.old"; do
    if [ -e "$stale" ]; then
        rm -rf "$stale"
        echo "  Removed stale: $(basename "$stale")"
    fi
done

# Remove legacy .json index files (replaced by .ruvector)
for legacy_json in "$TARGET/data/dist/lore.json" "$TARGET/data/dist/responses.json"; do
    if [ -f "$legacy_json" ]; then
        rm "$legacy_json"
        echo "  Removed legacy $(basename "$legacy_json") (using .ruvector)"
    fi
done

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

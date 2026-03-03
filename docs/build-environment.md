# ErenshorLLM Sidecar Build Environment

How to set up WSL2 Debian 12 to cross-compile the ErenshorLLM sidecar (Rust) for Windows x86_64.

The sidecar is a Rust binary that runs alongside the Erenshor game on Windows. It provides ONNX embedding inference, vector search, and LLM routing for the ErenshorLLMDialog BepInEx mod. Local LLM inference is handled by **shimmy**, a standalone pre-built binary that ships alongside the sidecar. Since development happens on WSL2 Linux, we cross-compile the sidecar for the `x86_64-pc-windows-gnu` target using MinGW.

---

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [System Requirements](#system-requirements)
- [Rust Toolchain](#rust-toolchain)
- [System Packages](#system-packages)
- [Build Configuration](#build-configuration)
- [Build Commands](#build-commands)
- [Runtime Dependencies](#runtime-dependencies)
- [Deployment](#deployment)
- [Known Issues](#known-issues)
- [Quick Setup Script](#quick-setup-script)

---

## Architecture Overview

```
Erenshor.exe (Unity game)
  |
  +-- BepInEx loads ErenshorLLMDialog.dll (C# mod)
        |
        +-- Spawns erenshor-llm.exe (Rust sidecar)
        |     - ONNX embedding (all-MiniLM-L6-v2)
        |     - Vector search (lore, templates, personality, memory)
        |     - LLM request routing (local/cloud/hybrid)
        |     - SONA adaptive learning
        |
        +-- Spawns shimmy.exe (local inference, when LLM mode = Local or Hybrid)
              - OpenAI-compatible HTTP server
              - Auto-discovers GGUF models from models/ directory
              - GPU auto-detection (CUDA, Vulkan, Metal, CPU fallback)
```

The sidecar itself is a pure Rust HTTP server with no C/C++ dependencies. All native code (LLM inference, GPU acceleration) is handled by shimmy as a separate process.

## System Requirements

- **Host**: WSL2 with Debian 12 (bookworm)
- **Target**: Windows x86_64 (`x86_64-pc-windows-gnu`)
- **Game**: Erenshor runs on Windows; the sidecar `.exe` is deployed alongside the BepInEx mod

## Rust Toolchain

Install Rust via [rustup](https://rustup.rs/) if not already present, then add the Windows cross-compilation target:

```bash
rustup target add x86_64-pc-windows-gnu
```

## System Packages

All installed via `sudo apt install`:

| Package | Purpose |
|---------|---------|
| `gcc-mingw-w64-x86-64` | MinGW GCC cross-compiler (linker for Windows target) |
| `g++-mingw-w64-x86-64` | MinGW G++ cross-compiler (static C++ stdlib linking) |

```bash
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64
```

No cmake, Vulkan SDK, libclang, or other heavy dependencies are needed. The previous llama-cpp-2 dependency required all of those; shimmy eliminates that complexity.

## Build Configuration

### `.cargo/config.toml`

Located at `sidecar/.cargo/config.toml`. Configures the Rust toolchain for cross-compilation:

```toml
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-g++-posix"
rustflags = ["-C", "target-feature=+crt-static", "-C", "link-args=-static-libgcc -static-libstdc++ -lstdc++ -lgomp -lwinpthread -static"]
```

**Key flags explained:**

| Setting | Purpose |
|---------|---------|
| `linker = "...g++-posix"` | Uses the posix-threading MinGW linker |
| `target-feature=+crt-static` | Static CRT linking |
| `-static-libgcc -static-libstdc++` | Statically link GCC runtimes to minimize runtime DLL dependencies |

## Build Commands

All build commands are run from the `sidecar/` directory.

### Using build.sh (recommended)

```bash
cd sidecar
./build.sh
```

This script:
1. Downloads shimmy v1.9.0 from GitHub releases (cached after first download)
2. Builds the native Linux binary (for vector index generation)
3. Cross-compiles the Windows binary
4. Builds lore and response template vector indexes
5. Deploys everything to the game directory

### Manual build

```bash
cd sidecar
cargo build --release --target x86_64-pc-windows-gnu
```

### Native build (for index generation and testing)

```bash
cd sidecar
cargo build --release --target-dir target-native
```

The native and cross-compile builds use separate target directories to prevent cargo cache contamination.

### Output locations

```
sidecar/target/x86_64-pc-windows-gnu/release/erenshor-llm.exe   # Windows binary
sidecar/target-native/release/erenshor-llm                       # Linux binary
sidecar/shimmy.exe                                                # Downloaded, not built
```

## Runtime Dependencies

The sidecar has minimal runtime dependencies compared to the previous llama-cpp-2 architecture:

| File | Source | Purpose |
|------|--------|---------|
| `erenshor-llm.exe` | Cross-compiled | Sidecar: embedding, vector search, LLM routing |
| `shimmy.exe` | [GitHub release](https://github.com/Michael-A-Kuykendall/shimmy/releases) | Local GGUF inference server (optional, only for Local/Hybrid mode) |
| `onnxruntime.dll` | Pre-built download | ONNX embedding model inference |

No MinGW runtime DLLs (`libstdc++-6.dll`, `libgomp-1.dll`, etc.) are required. The shimmy migration eliminated all C++ runtime dependencies from the sidecar.

## Deployment

Use `deploy.sh` from the ErenshorLLM project root to deploy both the C# mod DLL and the Rust sidecar binary with all dependencies:

```bash
# Using the $ErenshorGamePath environment variable (default)
./deploy.sh

# Explicit target directory
./deploy.sh /path/to/BepInEx/plugins/ErenshorLLMDialog
```

The deploy script copies the sidecar exe, shimmy exe, mod DLL, config files, vector indexes, embedding models, personality files, lore, and templates to the target directory. It preserves existing user config (`erenshor-llm.toml`) and places updated defaults alongside as `.default`.

## Known Issues

- **C# mod build on WSL.** The `dotnet build` command for the BepInEx mod requires explicit property flags because MSBuild doesn't resolve bash variables correctly through `-p:Key=$VAR` syntax. Use inline literal paths instead:
  ```bash
  dotnet build ErenshorLLMDialog.csproj \
      -p:BepInExPath="/mnt/d/SteamLibrary/steamapps/common/Erenshor/BepInEx/core" \
      -p:CorlibPath="/mnt/d/SteamLibrary/steamapps/common/Erenshor/Erenshor_Data/Managed"
  ```

- **PostBuild .bat errors on WSL are expected and harmless.** The DLL compiles successfully before the Windows batch file post-build step fails (WSL cannot execute `.bat` files). The build output is valid despite the error.

- **ONNX Runtime dynamic loading.** For native Linux builds (index generation), set `ORT_DYLIB_PATH` to point to the Linux `.so`:
  ```bash
  export ORT_DYLIB_PATH="$PWD/data/lib/libonnxruntime.so"
  ```
  The `build.sh` script sets this automatically.

## Quick Setup Script

For a fresh Debian 12 WSL2 environment:

```bash
# 1. Add Rust cross-compilation target
rustup target add x86_64-pc-windows-gnu

# 2. Install system packages (just MinGW, no cmake/vulkan/clang needed)
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64

# 3. Build everything (downloads shimmy, cross-compiles, builds indexes)
cd sidecar
./build.sh
```

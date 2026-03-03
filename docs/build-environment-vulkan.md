# ErenshorLLM Sidecar Build Environment

How to set up WSL2 Debian 12 to cross-compile the ErenshorLLM sidecar (Rust) for Windows x86_64 with optional Vulkan GPU support.

The sidecar is a Rust binary that runs alongside the Erenshor game on Windows. It provides local LLM inference (via llama.cpp), ONNX embedding inference, and vector search for the ErenshorLLMDialog BepInEx mod. Since development happens on WSL2 Linux, we cross-compile for the `x86_64-pc-windows-gnu` target using MinGW.

---

## Table of Contents

- [System Requirements](#system-requirements)
- [Rust Toolchain](#rust-toolchain)
- [System Packages](#system-packages)
- [Python Packages](#python-packages)
- [MinGW Threading Model](#mingw-threading-model)
- [Vulkan Cross-Compilation Setup](#vulkan-cross-compilation-setup)
- [Build Configuration Files](#build-configuration-files)
- [Build Commands](#build-commands)
- [Runtime Dependencies](#runtime-dependencies)
- [Deployment](#deployment)
- [Known Issues](#known-issues)
- [Quick Setup Script](#quick-setup-script)

---

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
| `gcc-mingw-w64-x86-64` | MinGW GCC cross-compiler for C |
| `g++-mingw-w64-x86-64` | MinGW G++ cross-compiler for C++ (required by llama.cpp) |
| `libvulkan-dev` | Vulkan headers and loader (platform-independent headers reused for mingw) |
| `glslc` | SPIR-V shader compiler (compiles Vulkan compute shaders at build time) |

```bash
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 libvulkan-dev glslc
```

## Python Packages

CMake is required by llama-cpp-sys-2's build process. If system cmake is not available (e.g., no sudo access at the time), install via pip:

```bash
pip install --user cmake --break-system-packages
```

The `--break-system-packages` flag is needed on Debian 12+ because PEP 668 marks the system Python as externally managed. This is safe for cmake since it is a standalone tool with no conflicting system package.

## MinGW Threading Model

MinGW ships with two threading model variants: **win32** and **posix**. The **posix** model is required because llama.cpp uses `std::thread` with callable arguments (e.g., `std::thread(func, arg1, arg2)`), which relies on `<thread>` header support that is only available with the posix threading model.

The build explicitly uses the posix-variant compilers:

- `x86_64-w64-mingw32-gcc-posix`
- `x86_64-w64-mingw32-g++-posix`

This is configured in `.cargo/config.toml` (linker) and `mingw-posix-toolchain.cmake` (cmake compilers via wrapper scripts).

## Vulkan Cross-Compilation Setup

Vulkan support enables GPU-accelerated LLM inference on AMD, Intel, and NVIDIA GPUs. The cross-compilation setup is non-trivial because we need to provide Vulkan SDK components for a Windows target from a Linux host.

### Symlinked Headers

Vulkan headers are platform-independent C headers. Rather than downloading the Windows Vulkan SDK, we symlink the Linux-installed headers into the MinGW sysroot:

```bash
sudo ln -s /usr/include/vulkan /usr/x86_64-w64-mingw32/include/vulkan
sudo ln -s /usr/include/vk_video /usr/x86_64-w64-mingw32/include/vk_video
```

### Vulkan Import Library

A Windows import library (`libvulkan-1.a`) for `vulkan-1.dll` is generated from the Vulkan headers. This library tells the linker which symbols exist in `vulkan-1.dll` without needing the actual DLL at link time:

```bash
# Generate a .def file listing all Vulkan API exports
echo "LIBRARY vulkan-1.dll" > /tmp/vulkan-1.def
echo "EXPORTS" >> /tmp/vulkan-1.def
grep -oP 'VKAPI_ATTR \w+ VKAPI_CALL (vk\w+)' /usr/include/vulkan/vulkan_core.h \
    | awk '{print $NF}' | sort -u >> /tmp/vulkan-1.def

# Generate the import library
x86_64-w64-mingw32-dlltool -d /tmp/vulkan-1.def -l sidecar/vulkan-sdk-mingw/Lib/libvulkan-1.a
```

### Fake Vulkan SDK Directory

The `llama-cpp-sys-2` crate's `build.rs` expects a `VULKAN_SDK` environment variable pointing to a directory with `Lib/` and `Include/` subdirectories (mimicking the LunarG Vulkan SDK layout). We create a minimal directory structure that satisfies this:

```
sidecar/vulkan-sdk-mingw/
+-- Lib/
|   +-- libvulkan-1.a           # Generated import library (see above)
+-- Include/
    +-- vulkan -> /usr/x86_64-w64-mingw32/include/vulkan   # Symlink
```

Create it with:

```bash
cd sidecar
mkdir -p vulkan-sdk-mingw/Lib vulkan-sdk-mingw/Include
ln -sf /usr/x86_64-w64-mingw32/include/vulkan vulkan-sdk-mingw/Include/vulkan
```

The `VULKAN_SDK` env var is set in `.cargo/config.toml` as a relative path.

## Build Configuration Files

### `.cargo/config.toml`

Located at `sidecar/.cargo/config.toml`. Configures the Rust toolchain for cross-compilation:

```toml
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-g++-posix"
rustflags = ["-C", "target-feature=+crt-static", "-C", "link-args=-static-libgcc -static-libstdc++ -lstdc++ -lgomp -lwinpthread -static"]

[env]
BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnu = "--sysroot=/usr/x86_64-w64-mingw32 -I/usr/lib/gcc/x86_64-w64-mingw32/12-posix/include"
CFLAGS_x86_64_pc_windows_gnu = "-D_WIN32_WINNT=0x0601"
CMAKE_TOOLCHAIN_FILE = { value = "mingw-posix-toolchain.cmake", relative = true }
VULKAN_SDK = { value = "vulkan-sdk-mingw", relative = true }
```

**Key flags explained:**

| Setting | Purpose |
|---------|---------|
| `linker = "...g++-posix"` | Uses the posix-threading MinGW linker (required by llama.cpp) |
| `target-feature=+crt-static` | Static CRT linking |
| `-static-libgcc -static-libstdc++` | Statically link GCC runtimes (see [Known Issues](#known-issues) for caveats) |
| `-lstdc++ -lgomp -lwinpthread` | Link C++ stdlib, OpenMP (llama.cpp parallelism), and pthreads |
| `BINDGEN_EXTRA_CLANG_ARGS` | Points bindgen (FFI generator) to the MinGW sysroot and GCC headers |
| `CFLAGS` with `-D_WIN32_WINNT=0x0601` | Sets Windows version to 7 (0x0601) to avoid referencing `THREAD_POWER_THROTTLING_STATE`, which is missing from Debian 12's MinGW headers (they lack Windows 11 SDK definitions) |
| `CMAKE_TOOLCHAIN_FILE` | Points CMake to the MinGW cross-compilation toolchain |
| `VULKAN_SDK` | Points to the fake Vulkan SDK directory for llama-cpp-sys-2's build.rs |

### `mingw-posix-toolchain.cmake`

Located at `sidecar/mingw-posix-toolchain.cmake`. Configures CMake for cross-compilation with MinGW. This file:

1. Sets the target system to Windows/x86_64
2. Uses wrapper scripts (`mingw-gcc-wrapper.sh`, `mingw-gxx-wrapper.sh`) as the C/C++ compilers
3. Restricts library and include search paths to the MinGW sysroot
4. Adds static linking flags for C++ runtime

```cmake
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
# Use wrapper scripts that strip MSVC flags (/FS etc) passed by llama-cpp-sys-2 build.rs
get_filename_component(_TOOLCHAIN_DIR "${CMAKE_CURRENT_LIST_FILE}" DIRECTORY)
set(CMAKE_C_COMPILER "${_TOOLCHAIN_DIR}/mingw-gcc-wrapper.sh")
set(CMAKE_CXX_COMPILER "${_TOOLCHAIN_DIR}/mingw-gxx-wrapper.sh")
set(CMAKE_RC_COMPILER x86_64-w64-mingw32-windres)
set(CMAKE_FIND_ROOT_PATH /usr/x86_64-w64-mingw32)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

# Force static linking of C++ runtime
set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -static")
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -static")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -static -static-libgcc -static-libstdc++")
set(CMAKE_SHARED_LINKER_FLAGS "${CMAKE_SHARED_LINKER_FLAGS} -static -static-libgcc -static-libstdc++")
```

### `mingw-gcc-wrapper.sh` / `mingw-gxx-wrapper.sh`

Located at `sidecar/mingw-gcc-wrapper.sh` and `sidecar/mingw-gxx-wrapper.sh`. These are wrapper scripts that filter out MSVC-specific compiler flags before forwarding to the actual MinGW compiler.

**Why they exist:** The `llama-cpp-sys-2` crate's `build.rs` unconditionally passes MSVC flags (`/FS`, `/EHsc`, `/MD`, `/MT`) to the C/C++ compiler, regardless of the target toolchain. GCC does not understand these flags and would fail. The wrappers silently strip them.

```bash
#!/bin/bash
# Example: mingw-gxx-wrapper.sh
ARGS=()
for arg in "$@"; do
    case "$arg" in
        /FS|/EHsc|/MD|/MT) ;; # skip MSVC flags
        *) ARGS+=("$arg") ;;
    esac
done
exec /usr/bin/x86_64-w64-mingw32-g++-posix "${ARGS[@]}"
```

Both wrapper scripts must be executable (`chmod +x`).

## Build Commands

All build commands are run from the `sidecar/` directory.

### CPU-only build (default)

```bash
cd sidecar
cargo build --release --target x86_64-pc-windows-gnu
```

No GPU acceleration. LLM inference runs entirely on CPU. Simplest to set up.

### Vulkan GPU build (recommended)

```bash
cd sidecar
cargo build --release --target x86_64-pc-windows-gnu --features vulkan
```

Enables GPU-accelerated inference via Vulkan. Works on AMD, Intel, and NVIDIA GPUs. Requires the [Vulkan cross-compilation setup](#vulkan-cross-compilation-setup) described above.

### CUDA GPU build (NVIDIA only)

```bash
cd sidecar
cargo build --release --target x86_64-pc-windows-gnu --features cuda
```

Enables NVIDIA-specific GPU acceleration via CUDA. Cross-compilation setup for CUDA is not yet documented and may require the CUDA toolkit installed in the MinGW sysroot.

### Output location

The compiled binary lands at:
```
sidecar/target/x86_64-pc-windows-gnu/release/erenshor-llm.exe
```

## Runtime Dependencies

The built `.exe` requires these DLLs alongside it at runtime. The `deploy.sh` script handles copying them.

| DLL | Source Path | Purpose |
|-----|-------------|---------|
| `libstdc++-6.dll` | `/usr/lib/gcc/x86_64-w64-mingw32/12-posix/` | C++ standard library |
| `libgcc_s_seh-1.dll` | `/usr/lib/gcc/x86_64-w64-mingw32/12-posix/` | GCC SEH runtime (structured exception handling) |
| `libgomp-1.dll` | `/usr/lib/gcc/x86_64-w64-mingw32/12-posix/` | OpenMP runtime (llama.cpp uses OpenMP for CPU parallelism) |
| `libwinpthread-1.dll` | `/usr/x86_64-w64-mingw32/lib/` | POSIX threading implementation for Windows |
| `vulkan-1.dll` | Ships with GPU drivers | Vulkan runtime (Vulkan builds only; not deployed by us) |
| `onnxruntime.dll` | Pre-built download | ONNX embedding model inference |

## Deployment

Use `deploy.sh` from the ErenshorLLM project root to deploy both the C# mod DLL and the Rust sidecar binary with all dependencies:

```bash
# Using the $ErenshorGamePath environment variable (default)
./deploy.sh

# Explicit target directory
./deploy.sh /path/to/BepInEx/plugins/ErenshorLLMDialog
```

The deploy script copies the sidecar exe, mod DLL, MinGW runtime DLLs, config files, vector indexes, embedding models, personality files, lore, and templates to the target directory. It preserves existing user config (`erenshor-llm.toml`) and places updated defaults alongside as `.default`.

## Known Issues

- **Static linking is incomplete.** Despite `-static-libgcc -static-libstdc++` flags in both `.cargo/config.toml` and the CMake toolchain, MinGW runtime DLLs are still required at runtime. This happens because llama.cpp's CMake-built object files embed dynamic references to the GCC runtime. The DLLs must be deployed alongside the exe.

- **Stale cmake caches between feature flag changes.** Switching between CPU and Vulkan builds (or vice versa) may require cleaning the target directory first, because CMake caches from the previous build can interfere:
  ```bash
  cargo clean --target x86_64-pc-windows-gnu
  ```

- **C# mod build on WSL.** The `dotnet build` command for the BepInEx mod has a pre-existing issue where MSBuild does not resolve `$(BepInExPath)` and `$(CorlibPath)` from environment variables. Use explicit `-p:` flags:
  ```bash
  dotnet build ErenshorLLMDialog.csproj \
      -p:GamePath="$ErenshorGamePath" \
      -p:BepInExPath="$ErenshorGamePath/BepInEx/core" \
      -p:CorlibPath="$ErenshorGamePath/Erenshor_Data/Managed"
  ```

- **PostBuild .bat errors on WSL are expected and harmless.** The DLL compiles successfully before the Windows batch file post-build step fails (WSL cannot execute `.bat` files). The build output is valid despite the error.

## Quick Setup Script

For a fresh Debian 12 WSL2 environment, run these commands to set up the complete build environment:

```bash
# 1. Add Rust cross-compilation target
rustup target add x86_64-pc-windows-gnu

# 2. Install system packages
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 libvulkan-dev glslc

# 3. Install cmake via pip (if system cmake not available)
pip install --user cmake --break-system-packages

# 4. Symlink Vulkan headers into MinGW sysroot
sudo ln -s /usr/include/vulkan /usr/x86_64-w64-mingw32/include/vulkan
sudo ln -s /usr/include/vk_video /usr/x86_64-w64-mingw32/include/vk_video

# 5. Generate Vulkan import library and fake SDK (run from sidecar/ directory)
cd sidecar
mkdir -p vulkan-sdk-mingw/Lib vulkan-sdk-mingw/Include
ln -sf /usr/x86_64-w64-mingw32/include/vulkan vulkan-sdk-mingw/Include/vulkan

echo "LIBRARY vulkan-1.dll" > /tmp/vulkan-1.def
echo "EXPORTS" >> /tmp/vulkan-1.def
grep -oP 'VKAPI_ATTR \w+ VKAPI_CALL (vk\w+)' /usr/include/vulkan/vulkan_core.h \
    | awk '{print $NF}' | sort -u >> /tmp/vulkan-1.def
x86_64-w64-mingw32-dlltool -d /tmp/vulkan-1.def -l vulkan-sdk-mingw/Lib/libvulkan-1.a

# 6. Make wrapper scripts executable
chmod +x mingw-gcc-wrapper.sh mingw-gxx-wrapper.sh

# 7. Build (Vulkan-enabled)
cargo build --release --target x86_64-pc-windows-gnu --features vulkan
```

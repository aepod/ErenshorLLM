#!/bin/bash
# Wrapper for x86_64-w64-mingw32-gcc-posix that strips MSVC flags
# (llama-cpp-sys-2 build.rs adds /FS which GCC doesn't understand)
ARGS=()
for arg in "$@"; do
    case "$arg" in
        /FS|/EHsc|/MD|/MT) ;; # skip MSVC flags
        *) ARGS+=("$arg") ;;
    esac
done
exec /usr/bin/x86_64-w64-mingw32-gcc-posix "${ARGS[@]}"

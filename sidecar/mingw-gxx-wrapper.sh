#!/bin/bash
# Wrapper for x86_64-w64-mingw32-g++-posix that strips MSVC flags
ARGS=()
for arg in "$@"; do
    case "$arg" in
        /FS|/EHsc|/MD|/MT) ;; # skip MSVC flags
        *) ARGS+=("$arg") ;;
    esac
done
exec /usr/bin/x86_64-w64-mingw32-g++-posix "${ARGS[@]}"

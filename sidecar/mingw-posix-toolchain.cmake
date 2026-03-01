set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
# Force host make for cross-compilation (cmake-rs uses "Unix Makefiles" generator)
set(CMAKE_MAKE_PROGRAM /usr/bin/make CACHE FILEPATH "" FORCE)
# Use wrapper scripts that strip MSVC flags (/FS etc) passed by llama-cpp-sys-2 build.rs
get_filename_component(_TOOLCHAIN_DIR "${CMAKE_CURRENT_LIST_FILE}" DIRECTORY)
set(CMAKE_C_COMPILER "${_TOOLCHAIN_DIR}/mingw-gcc-wrapper.sh")
set(CMAKE_CXX_COMPILER "${_TOOLCHAIN_DIR}/mingw-gxx-wrapper.sh")
set(CMAKE_RC_COMPILER x86_64-w64-mingw32-windres)
set(CMAKE_FIND_ROOT_PATH /usr/x86_64-w64-mingw32)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

# _WIN32_WINNT=0x0601 (Win7): skips THREAD_POWER_THROTTLING_STATE in ggml-cpu.c
# (the struct is missing from Debian 12 mingw headers).
# Force static linking of C++ runtime to avoid libstdc++-6.dll / libgomp-1.dll deps
set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -static -D_WIN32_WINNT=0x0601")
set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -static -D_WIN32_WINNT=0x0601")

# Disable cpp-httplib: it requires _WIN32_WINNT>=0x0A00 and we don't use the llama.cpp HTTP server
set(LLAMA_HTTPLIB OFF CACHE BOOL "" FORCE)
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -static -static-libgcc -static-libstdc++")
set(CMAKE_SHARED_LINKER_FLAGS "${CMAKE_SHARED_LINKER_FLAGS} -static -static-libgcc -static-libstdc++")

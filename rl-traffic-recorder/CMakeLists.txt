cmake_minimum_required(VERSION 3.20)
project(rl_traffic_recorder VERSION 1.0.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ── MinHook (vendored source — 5 .c files) ─────────────────────────────
add_library(minhook STATIC
    vendor/minhook/src/buffer.c
    vendor/minhook/src/hook.c
    vendor/minhook/src/trampoline.c
    vendor/minhook/src/HDE/hde64.c
    vendor/minhook/src/HDE/hde32.c
)
target_include_directories(minhook PUBLIC vendor/minhook)
# MinHook needs full windows.h for UINT_MAX etc.

# ── BakkesMod SDK (auto-clone if not present) ──────────────────────────
set(BAKKESMOD_SDK_DIR "")
if(EXISTS "${CMAKE_SOURCE_DIR}/bakkesmod/include/bakkesmod/plugin/bakkesmodplugin.h")
    set(BAKKESMOD_SDK_DIR "${CMAKE_SOURCE_DIR}/bakkesmod")
elseif(DEFINED ENV{LOCALAPPDATA})
    set(_bm_candidate "$ENV{LOCALAPPDATA}/bakkesmod/bakkesmod/bakkesmodsdk")
    if(EXISTS "${_bm_candidate}/include/bakkesmod/plugin/bakkesmodplugin.h")
        set(BAKKESMOD_SDK_DIR "${_bm_candidate}")
    endif()
endif()
if(NOT BAKKESMOD_SDK_DIR)
    message(STATUS "BakkesMod SDK not found locally — cloning from GitHub...")
    include(FetchContent)
    FetchContent_Declare(
        bakkesmod_sdk
        GIT_REPOSITORY https://github.com/bakkesmodorg/BakkesModSDK.git
        GIT_TAG        master
    )
    FetchContent_MakeAvailable(bakkesmod_sdk)
    set(BAKKESMOD_SDK_DIR "${bakkesmod_sdk_SOURCE_DIR}")
endif()
message(STATUS "BakkesMod SDK: ${BAKKESMOD_SDK_DIR}")

# ── Plugin DLL ─────────────────────────────────────────────────────────
add_library(rl_traffic_recorder SHARED
    src/dllmain.cpp
    src/pcap_writer.cpp
    src/json_logger.cpp
    src/winsock_hooks.cpp
    src/traffic_recorder.cpp
)

target_include_directories(rl_traffic_recorder PRIVATE
    ${CMAKE_SOURCE_DIR}/include
    ${BAKKESMOD_SDK_DIR}/include
)

target_link_libraries(rl_traffic_recorder PRIVATE
    minhook
    ws2_32
    ${BAKKESMOD_SDK_DIR}/lib/pluginsdk.lib
)

# Output as the name BakkesMod expects
set_target_properties(rl_traffic_recorder PROPERTIES
    OUTPUT_NAME "rl_traffic_recorder"
    SUFFIX ".dll"
    PREFIX ""
)

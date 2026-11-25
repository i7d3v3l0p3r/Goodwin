cmake_minimum_required(VERSION 3.23)
project(goodwin_wamr LANGUAGES C ASM)

set(WAMR_BUILD_INTERP 1)
set(WAMR_BUILD_FAST_INTERP 1)
set(WAMR_BUILD_AOT 0)
set(WAMR_BUILD_JIT 0)
set(WAMR_BUILD_FAST_JIT 0)
set(WAMR_BUILD_LIBC_BUILTIN 0)
set(WAMR_BUILD_LIBC_WASI 0)
set(WAMR_BUILD_LIB_PTHREAD 0)
set(WAMR_BUILD_THREAD_MGR 0)
set(WAMR_BUILD_MINI_LOADER 0)
set(WAMR_BUILD_MULTI_MEMORY 0)
set(WAMR_BUILD_MEMORY64 0)
set(WAMR_BUILD_SIMD 0)
set(WAMR_BUILD_BULK_MEMORY 1)
set(WAMR_BUILD_REF_TYPES 1)
set(WAMR_BUILD_INSTRUCTION_METERING 1)
set(WAMR_BUILD_ALLOC_WITH_USAGE 1)
set(WAMR_BUILD_SHRUNK_MEMORY 0)
set(WAMR_DISABLE_HW_BOUND_CHECK 1)
set(WAMR_BUILD_LOG 0)
include("${WAMR_ROOT_DIR}/build-scripts/runtime_lib.cmake")
find_package(Git REQUIRED)

function(patch_sources patch)
    set(patched "${CMAKE_CURRENT_BINARY_DIR}/patched")
    foreach(relative IN LISTS ARGN)
        set(original "${WAMR_ROOT_DIR}/${relative}")
        get_filename_component(directory "${patched}/${relative}" DIRECTORY)
        file(MAKE_DIRECTORY "${directory}")
        file(COPY_FILE "${original}" "${patched}/${relative}")
        list(REMOVE_ITEM WAMR_RUNTIME_LIB_SOURCE "${original}")
        list(APPEND WAMR_RUNTIME_LIB_SOURCE "${patched}/${relative}")
    endforeach()
    execute_process(
        COMMAND "${GIT_EXECUTABLE}" apply --no-index "${CMAKE_CURRENT_SOURCE_DIR}/${patch}"
        WORKING_DIRECTORY "${patched}"
        COMMAND_ERROR_IS_FATAL ANY
    )
    set(WAMR_RUNTIME_LIB_SOURCE "${WAMR_RUNTIME_LIB_SOURCE}" PARENT_SCOPE)
endfunction()

patch_sources(table-limit.patch core/iwasm/interpreter/wasm_loader.c)
if(WAMR_BUILD_PLATFORM STREQUAL "esp-idf")
    patch_sources(esp-idf.patch
        core/shared/platform/esp-idf/espidf_platform.c
        core/shared/platform/esp-idf/espidf_thread.c)
endif()

add_library(goodwin_wamr STATIC ${WAMR_RUNTIME_LIB_SOURCE} bridge.c)
target_compile_definitions(goodwin_wamr PRIVATE WASM_TABLE_MAX_SIZE=4096)
target_compile_options(goodwin_wamr PRIVATE -Os -ffunction-sections -fdata-sections)
target_include_directories(goodwin_wamr PRIVATE
    "${WAMR_ROOT_DIR}/core/iwasm/include"
    "${WAMR_ROOT_DIR}/core/iwasm/interpreter")
if(WAMR_BUILD_PLATFORM STREQUAL "linux")
    set_property(TARGET goodwin_wamr PROPERTY POSITION_INDEPENDENT_CODE ON)
endif()

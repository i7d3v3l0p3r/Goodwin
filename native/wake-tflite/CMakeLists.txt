cmake_minimum_required(VERSION 3.23)
project(goodwin_wake_tflite LANGUAGES C CXX)

set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/lib")
set(TFLM_ENABLE_SIGNAL OFF CACHE BOOL "" FORCE)
set(TFLM_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(TFLM_INSTALL OFF CACHE BOOL "" FORCE)
set(TFLM_REMOVE_VIRTUAL_DELETE ON CACHE BOOL "" FORCE)
add_subdirectory("${TFLM_SOURCE}" tflm)

add_library(goodwin_wake_tflite STATIC bridge.cc "${LIVEKIT_SOURCE}/lkww_frontend.c")
target_link_libraries(goodwin_wake_tflite PRIVATE tflm::micro)
target_include_directories(goodwin_wake_tflite PRIVATE "${LIVEKIT_SOURCE}")
target_compile_options(goodwin_wake_tflite PRIVATE
    $<$<COMPILE_LANGUAGE:CXX>:-fno-exceptions;-fno-rtti;-fno-threadsafe-statics>
    -ffunction-sections -fdata-sections)
target_compile_features(goodwin_wake_tflite PRIVATE cxx_std_17)

cmake_minimum_required(VERSION 3.30)
project(nvidia_nrd_bridge LANGUAGES CXX)

if(NOT NRD_SOURCE_DIR)
    message(FATAL_ERROR "NRD_SOURCE_DIR is required")
endif()

set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(NRD_STATIC_LIBRARY ON CACHE BOOL "" FORCE)
set(NRD_NRI ON CACHE BOOL "" FORCE)
set(NRD_EMBEDS_SPIRV_SHADERS ON CACHE BOOL "" FORCE)
set(NRD_EMBEDS_DXIL_SHADERS OFF CACHE BOOL "" FORCE)
set(NRD_EMBEDS_DXBC_SHADERS OFF CACHE BOOL "" FORCE)
set(NRD_NORMAL_ENCODING 4 CACHE STRING "" FORCE)
set(NRD_ROUGHNESS_ENCODING 1 CACHE STRING "" FORCE)
set(NRD_SHADERS_PATH "${CMAKE_CURRENT_BINARY_DIR}/nrd-shaders" CACHE PATH "" FORCE)
set(NRI_STATIC_LIBRARY ON CACHE BOOL "" FORCE)
set(NRI_ENABLE_VK_SUPPORT ON CACHE BOOL "" FORCE)
set(NRI_ENABLE_NONE_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_VALIDATION_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_D3D11_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_D3D12_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_NVTX_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_DEBUG_NAMES_AND_ANNOTATIONS OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_XLIB_SUPPORT OFF CACHE BOOL "" FORCE)
set(NRI_ENABLE_WAYLAND_SUPPORT OFF CACHE BOOL "" FORCE)
set(SHADERMAKE_FIND_DXC OFF CACHE BOOL "" FORCE)
set(SHADERMAKE_FIND_DXC_VK ON CACHE BOOL "" FORCE)
set(FETCHCONTENT_FULLY_DISCONNECTED ON CACHE BOOL "" FORCE)
set(FETCHCONTENT_SOURCE_DIR_SSE2NEON "${NRD_SOURCE_DIR}/deps/sse2neon" CACHE PATH "" FORCE)
set(FETCHCONTENT_SOURCE_DIR_VULKAN_HEADERS "${NRD_SOURCE_DIR}/deps/vulkan-headers" CACHE PATH "" FORCE)
set(FETCHCONTENT_SOURCE_DIR_VMA "${NRD_SOURCE_DIR}/deps/vma" CACHE PATH "" FORCE)

if(MSVC)
    add_compile_options(/EHsc)
endif()

add_subdirectory("${NRD_SOURCE_DIR}/deps/mathlib" mathlib)
add_subdirectory("${NRD_SOURCE_DIR}/deps/shadermake" shadermake)
add_subdirectory("${NRD_SOURCE_DIR}/deps/nri" nri)
add_subdirectory("${NRD_SOURCE_DIR}" nrd)

# NRI v179 promotes these compiler-dependent diagnostics to errors.
foreach(target NRI NRI_Shared NRI_VK)
    if(CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
        target_compile_options(${target} PRIVATE -Wno-error=maybe-uninitialized)
    elseif(CMAKE_CXX_COMPILER_ID MATCHES "Clang")
        target_compile_options(${target} PRIVATE -Wno-error=inconsistent-missing-override)
    endif()
endforeach()

add_library(nvidia_nrd STATIC nrd.cpp)
target_compile_features(nvidia_nrd PRIVATE cxx_std_17)
target_include_directories(nvidia_nrd SYSTEM PRIVATE
    "${NRD_SOURCE_DIR}/Include"
    "${NRD_SOURCE_DIR}/Integration"
    "${NRD_SOURCE_DIR}/deps/nri/Include"
)
target_link_libraries(nvidia_nrd PRIVATE NRD NRI)
if(MSVC)
    target_compile_options(nvidia_nrd PRIVATE /W4 /WX /EHsc)
else()
    target_compile_options(nvidia_nrd PRIVATE -Wall -Wextra -Werror -fPIC)
endif()

install(TARGETS nvidia_nrd NRD NRI NRI_Shared NRI_VK ShaderMakeBlob
    ARCHIVE DESTINATION lib
)

# Builds an AUv2 (.component) wrapper around the already-built CLAP bundle,
# using https://github.com/free-audio/clap-wrapper.
#
# clap-wrapper consumes the compiled .clap binary, not our source, so nothing
# here is tied to nice-plug (or to nih-plug, which the original write-up of this
# approach targeted). Any spec-compliant CLAP bundle works the same way.
#
# Usage, from the workspace root, after the CLAP bundle exists:
#
#   cargo xtask bundle-universal rp2a03_niceplug --release \
#       --no-default-features --features wgpu
#   cmake -B target/auv2-build -S packaging/auv2
#   cmake --build target/auv2-build --config Release
#
# Result: target/auv2-build/RP2A03 Synth.component

cmake_minimum_required(VERSION 3.21)

# Both of these must be set before clap-wrapper's project() runs — it only fills
# them in itself if they are still undefined at that point.
#
# Architectures: without this clap-wrapper does a native-only build, and the
# .component would not match the universal .clap it wraps.
if(NOT DEFINED CMAKE_OSX_ARCHITECTURES)
    set(CMAKE_OSX_ARCHITECTURES "x86_64;arm64" CACHE STRING "AU bundle architectures" FORCE)
endif()

# Deployment target: clap-wrapper's own default is 10.13, which predates Apple
# Silicon and so can't be honoured by the arm64 slice.
if(NOT DEFINED CMAKE_OSX_DEPLOYMENT_TARGET)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "Minimum macOS version" FORCE)
endif()

project(rp2a03_synth_auv2 LANGUAGES C CXX)

# The AudioUnit SDK is Objective-C++; these three lines are what make it build.
set(CMAKE_CXX_STANDARD 17)
enable_language(OBJC)
enable_language(OBJCXX)

set(RP2A03_CLAP_PATH "${CMAKE_CURRENT_SOURCE_DIR}/../../target/bundled/rp2a03_niceplug.clap"
    CACHE PATH "Path to the built .clap bundle to wrap")
set(RP2A03_VERSION "0.0.6" CACHE STRING "Bundle version (numeric; no -Alpha suffix)")

if(NOT EXISTS "${RP2A03_CLAP_PATH}")
    message(FATAL_ERROR
        "CLAP bundle not found at '${RP2A03_CLAP_PATH}'.\n"
        "Build it first with: cargo xtask bundle-universal rp2a03_niceplug --release "
        "--no-default-features --features wgpu")
endif()

# Let clap-wrapper fetch the CLAP headers and AudioUnitSDK itself via CPM.
set(CLAP_WRAPPER_DOWNLOAD_DEPENDENCIES ON CACHE BOOL "" FORCE)
set(CLAP_WRAPPER_BUILD_TESTS OFF CACHE BOOL "" FORCE)

# FetchContent rather than a git submodule: keeps the tree clean and pins the
# wrapper to a release tag instead of whatever the submodule happens to point at.
include(FetchContent)
FetchContent_Declare(clap-wrapper
    GIT_REPOSITORY https://github.com/free-audio/clap-wrapper.git
    GIT_TAG v0.15.1
    GIT_SHALLOW ON)
FetchContent_MakeAvailable(clap-wrapper)

set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")

add_library(rp2a03_synth_auv2 MODULE)

target_add_auv2_wrapper(
    TARGET rp2a03_synth_auv2
    # clap-wrapper copies the .clap bundle inside the .component, so the
    # resulting AU is self-contained.
    MACOS_EMBEDDED_CLAP_LOCATION "${RP2A03_CLAP_PATH}"

    OUTPUT_NAME "RP2A03 Synth"
    BUNDLE_IDENTIFIER "com.rp2a03.synth.auv2"
    BUNDLE_VERSION "${RP2A03_VERSION}"

    MANUFACTURER_NAME "PurpBatBoi"
    # Both codes are exactly 4 characters and are not all-lowercase — Apple
    # reserves all-lowercase codes for itself.
    MANUFACTURER_CODE "PbBi"
    SUBTYPE_CODE "R2a3"

    # aumu = music device (instrument). The commonly-linked write-up of this
    # approach uses "aufx", but that is an *effect* type and would be wrong for
    # a synth. clap-wrapper would infer aumu anyway from our CLAP_FEATURES
    # starting with ClapFeature::Instrument; setting it keeps that correct even
    # if the feature list is ever reordered.
    INSTRUMENT_TYPE "aumu"
)

# Builds an AUv3 wrapper around the already-built CLAP bundle, using
# https://github.com/free-audio/clap-wrapper.
#
# clap-wrapper consumes the compiled .clap binary, not our source, so nothing
# here is tied to nice-plug (or to nih-plug, which the original write-up of this
# approach targeted). Any spec-compliant CLAP bundle works the same way.
#
# AUv3, unlike AUv2, is an *app extension*: the plugin proper is a .appex, and
# macOS only discovers it when it is embedded inside a containing .app that the
# user has run at least once. So this project builds two targets:
#
#   rp2a03_synth_auv3     -> RP2A03 Synth.appex          (the plugin)
#   rp2a03_synth_auv3_sa  -> RP2A03 Synth AUv3.app       (the container/host)
#
# Only the .app ships; the .appex lives inside it at Contents/PlugIns/.
#
# Usage, from the workspace root, after the CLAP bundle exists.  Note -G Xcode:
# clap-wrapper hard-requires the Xcode generator for AUv3, because the .appex
# must be linked as an app-extension product (entry point _NSExtensionMain) and
# signed by Xcode -- Ninja and Makefiles cannot produce a loadable appex.
#
#   cargo xtask bundle-universal rp2a03_niceplug --release \
#       --no-default-features --features wgpu
#   cmake -B target/auv3-build -S packaging/auv3 -G Xcode
#   cmake --build target/auv3-build --config Release
#
# Result: target/auv3-build/Release/RP2A03 Synth AUv3.app

cmake_minimum_required(VERSION 3.21)

if(NOT CMAKE_GENERATOR STREQUAL "Xcode")
    message(FATAL_ERROR
        "AUv3 requires the Xcode generator. Re-run with:\n"
        "  cmake -B <builddir> -S packaging/auv3 -G Xcode")
endif()

# Both of these must be set before clap-wrapper's project() runs — it only fills
# them in itself if they are still undefined at that point.
#
# Architectures: without this clap-wrapper does a native-only build, and the
# bundle would not match the universal .clap it wraps.
if(NOT DEFINED CMAKE_OSX_ARCHITECTURES)
    set(CMAKE_OSX_ARCHITECTURES "x86_64;arm64" CACHE STRING "AU bundle architectures" FORCE)
endif()

# Deployment target: clap-wrapper's own default is 10.13, which predates Apple
# Silicon and so can't be honoured by the arm64 slice.
if(NOT DEFINED CMAKE_OSX_DEPLOYMENT_TARGET)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "11.0" CACHE STRING "Minimum macOS version" FORCE)
endif()

project(rp2a03_synth_auv3 LANGUAGES C CXX)

# The AudioUnit/AVFoundation headers are Objective-C++; these lines are what
# make them build.
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

# Let clap-wrapper fetch the CLAP headers itself via CPM. AUv3 does not need the
# AudioUnit SDK (it uses AudioToolbox.framework directly), unlike AUv2.
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

# The .appex. add_executable, not add_library(MODULE): an app extension is an
# executable product. It is left source-less on purpose — the wrapper attaches
# its own sources.
add_executable(rp2a03_synth_auv3)

target_add_auv3_wrapper(
    TARGET rp2a03_synth_auv3
    # clap-wrapper copies the .clap bundle inside the bundle, so the resulting
    # AU is self-contained. Passing this rather than CLAP_TARGET_FOR_CONFIG puts
    # us on clap-wrapper's "nontarget" path, which is what makes the three
    # MANUFACTURER_*/SUBTYPE_CODE arguments below mandatory.
    MACOS_EMBEDDED_CLAP_LOCATION "${RP2A03_CLAP_PATH}"

    OUTPUT_NAME "RP2A03 Synth"
    BUNDLE_IDENTIFIER "com.rp2a03.synth.auv3"
    BUNDLE_VERSION "${RP2A03_VERSION}"

    MANUFACTURER_NAME "PurpBatBoi"
    # Both codes are exactly 4 characters and are not all-lowercase — Apple
    # reserves all-lowercase codes for itself.
    MANUFACTURER_CODE "PbBi"
    SUBTYPE_CODE "R2a3"

    # aumu = music device (instrument). clap-wrapper defaults to aumu with a
    # warning if this is omitted; setting it explicitly keeps that correct and
    # quiet. "aufx" would be an *effect*, which this is not.
    INSTRUMENT_TYPE "aumu"
)

# The container .app. macOS registers the AUv3 from the copy of the .appex that
# this app carries in Contents/PlugIns/, so this is the artifact users install.
add_executable(rp2a03_synth_auv3_sa)

target_add_auv3_standalone_wrapper(
    TARGET rp2a03_synth_auv3_sa
    AUV3_TARGET rp2a03_synth_auv3

    OUTPUT_NAME "RP2A03 Synth AUv3"
    BUNDLE_IDENTIFIER "com.rp2a03.synth.auv3standalone"
    BUNDLE_VERSION "${RP2A03_VERSION}"

    # Must match the codes given to the wrapper above, or the host app and the
    # extension it carries would advertise two different plugins.
    AU_TYPE "aumu"
    AU_SUBTYPE "R2a3"
    AU_MANUFACTURER "PbBi"
)

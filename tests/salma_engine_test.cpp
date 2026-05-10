#include "SalmaEngine.hpp"

#include <gtest/gtest.h>

#include <filesystem>
#include <stdexcept>
#include <string>

namespace fs = std::filesystem;

// The bridge from the Crow server to the Rust engine DLL. These tests are the
// only coverage of that boundary. Every dashboard install, infer and resolve
// request crosses it, and a broken DLL load surfaces here instead of as a
// runtime 500 with an empty body.
//
// Contracts pinned here, one test each:
//   1. LoadsTheEngineDll        - ensure_loaded() finds and loads mo2-salma.dll
//                                 and reports the path it used.
//   2. ReportsTheAbiVersion     - getApiVersion matches MO2_SALMA_API_VERSION
//                                 in src/capi.rs, which CMake also parses.
//   3. InferReturnsEmpty...     - inference collapses every failure to "".
//   4. ResolveReturnsEmpty...   - resolution collapses every failure to an
//                                 empty path.
//   5. InstallThrowsOnFailure...- a failed install throws, because the Crow
//                                 controllers catch std::exception while the
//                                 flat ABI only returns an error string.
//
// Not covered: the mutex that serializes install_mod against the engine's
// process-global `installSucceeded` flag (g_install_mutex in
// src/SalmaEngine.cpp). Every test below is single-threaded, so deleting that
// mutex would leave this suite green. CLAUDE.md says this file covers both of
// SalmaEngine's restored behaviors; only the throw is covered. Add a
// concurrent-install test before trusting that claim.
//
// These tests need mo2-salma.dll beside the test executable. CMake copies it
// from target/package/ as a post-build step, so a plain `cmake --build` is
// enough. With no packaged engine they fail loudly rather than skipping,
// because a server that cannot load its engine is not a working server.

TEST(SalmaEngine, LoadsTheEngineDll)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded())
        << "mo2-salma.dll did not load. Run build.bat (or python scripts/package.py) "
           "so target/package/mo2-salma.dll exists, then rebuild.";
    EXPECT_FALSE(mo2server::SalmaEngine::loaded_path().empty());
}

TEST(SalmaEngine, ReportsTheAbiVersion)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // Bump this together with MO2_SALMA_API_VERSION in src/capi.rs; CMake
    // parses the same constant for the release bundle name.
    EXPECT_EQ(mo2server::SalmaEngine::api_version(), "1.2.0");
}

TEST(SalmaEngine, InferReturnsEmptyForMissingPaths)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // Inference has no error channel. Every failure comes back as "".
    EXPECT_EQ(mo2server::SalmaEngine::infer_selections("no-such-archive.7z", "no-such-mod"), "");
}

TEST(SalmaEngine, ResolveReturnsEmptyPathWhenNothingMatches)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    auto resolved = mo2server::SalmaEngine::resolve_mod_archive(
        "no-such-installation-file.7z", fs::path("no-such-mod"), fs::path("no-such-mods-dir"));
    EXPECT_TRUE(resolved.empty());
}

TEST(SalmaEngine, InstallThrowsOnFailureLikeTheOldCppService)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // The controllers catch std::exception around this call. That only works
    // because the bridge turns the engine's error return into a throw; the flat
    // ABI itself never raises one.
    EXPECT_THROW(
        {
            (void)mo2server::SalmaEngine::install_mod("no-such-archive.7z", "no-such-mod", "");
        },
        std::runtime_error);
}

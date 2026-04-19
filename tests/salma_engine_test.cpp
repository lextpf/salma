#include "SalmaEngine.hpp"

#include <gtest/gtest.h>

#include <filesystem>
#include <stdexcept>
#include <string>

namespace fs = std::filesystem;

// The bridge from the Crow server to the Rust engine DLL. These tests are the
// only coverage of that boundary: every dashboard install/infer/resolve request
// goes through it, and a broken DLL load surfaces here rather than as a runtime
// 500 with an empty body.
//
// They require mo2-salma.dll beside the test executable. CMake copies it there
// from target/package/ as a post-build step, so a plain `cmake --build` is
// enough; if the Rust engine has never been packaged, these fail loudly rather
// than silently skipping, because a server that cannot load its engine is not a
// working server.

TEST(SalmaEngine, LoadsTheEngineDll)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded())
        << "mo2-salma.dll did not load. Run build.bat (or python tools/package.py) "
           "so target/package/mo2-salma.dll exists, then rebuild.";
    EXPECT_FALSE(mo2server::SalmaEngine::loaded_path().empty());
}

TEST(SalmaEngine, ReportsTheAbiVersion)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // Must match MO2_SALMA_API_VERSION in src/capi.rs, which CMake also parses
    // for the release bundle name.
    EXPECT_EQ(mo2server::SalmaEngine::api_version(), "1.2.0");
}

TEST(SalmaEngine, InferReturnsEmptyForMissingPaths)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // Mirror of the engine contract: inference collapses every failure to "".
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
    // The controllers catch std::exception around this call, which only works
    // because the bridge converts the engine's error return into a throw the
    // way the former in-process InstallationService did.
    EXPECT_THROW(
        {
            (void)mo2server::SalmaEngine::install_mod("no-such-archive.7z", "no-such-mod", "");
        },
        std::runtime_error);
}

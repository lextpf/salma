#include "SalmaEngine.hpp"

#include <gtest/gtest.h>

#include <filesystem>
#include <stdexcept>
#include <string>

namespace fs = std::filesystem;

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
    // keep this value synchronized with MO2_SALMA_API_VERSION in capi.rs.
    EXPECT_EQ(mo2server::SalmaEngine::api_version(), "1.2.0");
}

TEST(SalmaEngine, InferReturnsEmptyForMissingPaths)
{
    ASSERT_TRUE(mo2server::SalmaEngine::ensure_loaded());
    // inference reports failure as empty text.
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
    // the bridge converts an engine error result to an exception.
    EXPECT_THROW(
        {
            (void)mo2server::SalmaEngine::install_mod("no-such-archive.7z", "no-such-mod", "");
        },
        std::runtime_error);
}

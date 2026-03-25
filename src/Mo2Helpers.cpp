#include "Mo2Helpers.h"

#include "Logger.h"

#include <cstdlib>
#include <filesystem>
#include <format>

namespace mo2server
{

std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path)
{
    if (const char* deploy_env = std::getenv("SALMA_DEPLOY_PATH"); deploy_env && *deploy_env)
    {
        return std::filesystem::path(deploy_env);
    }

    std::filesystem::path mods_path = mo2_mods_path;
    if (mods_path.empty())
    {
        if (const char* mods_env = std::getenv("SALMA_MODS_PATH"); mods_env && *mods_env)
        {
            mods_path = std::filesystem::path(mods_env);
        }
    }

    if (!mods_path.empty())
    {
        auto instance_root = mods_path.parent_path().parent_path();
        if (!instance_root.empty())
        {
            return instance_root / "MO2" / "plugins";
        }
    }

    mo2core::Logger::instance().log_error(
        "[server] Cannot determine plugin deploy path: "
        "set SALMA_DEPLOY_PATH or configure MO2 mods path");
    return {};
}

bool plugin_installed_at(const std::filesystem::path& deploy_path)
{
    if (deploy_path.empty())
    {
        return false;
    }
    try
    {
        return std::filesystem::exists(deploy_path / "salma" / "mo2-salma.dll") &&
               std::filesystem::exists(deploy_path / "mo2-salma.py");
    }
    catch (const std::exception& ex)
    {
        mo2core::Logger::instance().log_warning(std::format(
            "[server] Error checking plugin install at {}: {}", deploy_path.string(), ex.what()));
        return false;
    }
}

}  // namespace mo2server

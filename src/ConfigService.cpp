#include "ConfigService.h"
#include "Logger.h"

#include <format>
#include <fstream>
#include <nlohmann/json.hpp>

#ifdef _WIN32
#include <windows.h>
#endif

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

ConfigService::ConfigService()
{
    // salma.json lives next to the executable
    wchar_t exe_path[MAX_PATH];
    GetModuleFileNameW(nullptr, exe_path, MAX_PATH);
    config_path_ = fs::path(exe_path).parent_path() / "salma.json";
}

ConfigService& ConfigService::instance()
{
    static ConfigService inst;
    return inst;
}

void ConfigService::load()
{
    std::lock_guard lock(mutex_);
    auto& logger = mo2core::Logger::instance();

    if (!fs::exists(config_path_))
    {
        logger.log(
            std::format("[server] No config file at {}, using defaults", config_path_.string()));
        return;
    }

    try
    {
        std::ifstream ifs(config_path_);
        auto j = json::parse(ifs);
        mo2_mods_path_ = j.value("mo2ModsPath", "");
        logger.log(std::format("[server] Config loaded: mods={}", mo2_mods_path_));
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[server] Failed to load config: {}", ex.what()));
    }
}

void ConfigService::save()
{
    std::lock_guard lock(mutex_);
    auto& logger = mo2core::Logger::instance();

    try
    {
        json j = {{"mo2ModsPath", mo2_mods_path_}};
        std::ofstream ofs(config_path_);
        ofs << j.dump(2);
        logger.log("[server] Config saved");
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[server] Failed to save config: {}", ex.what()));
    }
}

std::string ConfigService::mo2_mods_path() const
{
    std::lock_guard lock(mutex_);
    return mo2_mods_path_;
}

void ConfigService::set_mo2_mods_path(const std::string& path)
{
    std::lock_guard lock(mutex_);
    mo2_mods_path_ = path;
}

fs::path ConfigService::fomod_output_dir() const
{
    std::lock_guard lock(mutex_);
    if (mo2_mods_path_.empty())
        return {};
    return fs::path(mo2_mods_path_) / "Salma FOMODs Output" / "fomods";
}

fs::path ConfigService::config_path() const
{
    return config_path_;
}

}  // namespace mo2server

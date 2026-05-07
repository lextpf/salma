#include "ConfigService.hpp"
#include "Logger.hpp"
#include "Utils.hpp"

#include <format>
#include <fstream>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

// ConfigService - salma.json, the server's only persisted state.
//
// mo2ModsPath is the single key written. fomod_output_dir() is derived from it
// and never stored, so moving the mods path moves the output directory with it.
//
// Reads are forgiving: a missing file, an unopenable file and a parse error all
// leave the defaults in place and log, because the dashboard must still start
// so the user can fix the path. Writes are not: save() reports failure and
// apply_mo2_mods_path() rolls the in-memory value back, so memory and disk
// never disagree. ConfigService.hpp documents the locking.

namespace mo2server
{

ConfigService::ConfigService()
{
    // Anchor salma.json to the executable, not the working directory, so the
    // file is the same one whichever directory the server was launched from.
    // executable_directory() falls back to the working directory only if the
    // Win32 lookup fails.
    config_path_ = mo2core::executable_directory() / "salma.json";
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
        if (!ifs.is_open())
        {
            logger.log_error(std::format("[server] Cannot open config: {}", config_path_.string()));
            return;
        }
        auto j = json::parse(ifs);
        mo2_mods_path_ = j.value("mo2ModsPath", "");
        if (!mo2_mods_path_.empty())
        {
            std::error_code ec;
            const bool valid = fs::is_directory(mo2_mods_path_, ec);
            if (!valid || ec)
            {
                logger.log_warning(
                    std::format("[server] Config loaded but mo2ModsPath does not point at an "
                                "existing directory: {}",
                                mo2_mods_path_));
            }
        }
        logger.log(std::format("[server] Config loaded: mods={}", mo2_mods_path_));
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[server] Failed to load config: {}", ex.what()));
    }
}

bool ConfigService::save()
{
    std::lock_guard lock(mutex_);
    auto& logger = mo2core::Logger::instance();

    // Write a sibling temp file, then rename over the target. On MSVC
    // std::filesystem::rename uses MoveFileExW with REPLACE_EXISTING, so no
    // reader ever sees a partially written salma.json.
    auto tmp_path = config_path_;
    tmp_path += ".tmp";

    try
    {
        json j = {{"mo2ModsPath", mo2_mods_path_}};
        {
            std::ofstream ofs(tmp_path, std::ios::binary | std::ios::trunc);
            if (!ofs.is_open())
            {
                logger.log_error(std::format("[server] Cannot open config tempfile for writing: {}",
                                             tmp_path.string()));
                return false;
            }
            ofs << j.dump(2);
            ofs.flush();
            if (!ofs.good())
            {
                logger.log_warning("[server] Failed to write config tempfile");
                std::error_code ignore;
                fs::remove(tmp_path, ignore);
                return false;
            }
        }

        std::error_code ec;
        fs::rename(tmp_path, config_path_, ec);
        if (ec)
        {
            logger.log_error(std::format("[server] Atomic rename failed for config tempfile {}: {}",
                                         tmp_path.string(),
                                         ec.message()));
            std::error_code ignore;
            fs::remove(tmp_path, ignore);
            return false;
        }
        logger.log("[server] Config saved");
        return true;
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[server] Failed to save config: {}", ex.what()));
        std::error_code ignore;
        fs::remove(tmp_path, ignore);
        return false;
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

bool ConfigService::apply_mo2_mods_path(const std::string& path)
{
    std::string previous;
    {
        std::lock_guard lock(mutex_);
        previous = mo2_mods_path_;
        mo2_mods_path_ = path;
    }
    if (save())
    {
        return true;
    }
    // The save failed, so roll the in-memory value back. Otherwise the running
    // process would serve a path that the next restart discards.
    std::lock_guard lock(mutex_);
    mo2_mods_path_ = previous;
    return false;
}

bool ConfigService::is_mo2_mods_path_valid() const
{
    std::lock_guard lock(mutex_);
    if (mo2_mods_path_.empty())
    {
        return false;
    }
    std::error_code ec;
    return fs::is_directory(mo2_mods_path_, ec) && !ec;
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

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
    // salma.json lives next to the executable.
    // Retry with larger buffer if MAX_PATH is insufficient (long path support).
    std::wstring exe_buf(MAX_PATH, L'\0');
    DWORD len = GetModuleFileNameW(nullptr, exe_buf.data(), static_cast<DWORD>(exe_buf.size()));
    constexpr int kMaxRetries = 5;
    int retries = 0;
    while (len >= exe_buf.size() && retries < kMaxRetries)
    {
        exe_buf.resize(exe_buf.size() * 2);
        len = GetModuleFileNameW(nullptr, exe_buf.data(), static_cast<DWORD>(exe_buf.size()));
        ++retries;
    }
    if (len == 0 || retries >= kMaxRetries)
    {
        config_path_ = fs::current_path() / "salma.json";
    }
    else
    {
        exe_buf.resize(len);
        config_path_ = fs::path(exe_buf).parent_path() / "salma.json";
    }
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

    // Write to a sibling temp file then rename atomically over the target.
    // std::filesystem::rename on MSVC uses MoveFileExW with REPLACE_EXISTING,
    // so callers and future reloaders never observe a partially-written file.
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
    // Save failed -- roll the in-memory value back so the running process and
    // disk agree on the previous configuration.
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

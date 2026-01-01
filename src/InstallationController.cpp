#include "InstallationController.h"
#include "ConfigService.h"
#include "InstallationService.h"
#include "Logger.h"
#include "MultipartHandler.h"
#include "Utils.h"

#include <algorithm>
#include <filesystem>
#include <format>
#include <fstream>
#include <regex>
#include <thread>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#endif

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

static crow::response json_response(int status, const json& body)
{
    crow::response res(status, body.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

static std::string strip_nexus_suffix(const std::string& stem)
{
    // Nexus-style suffix: Name-ModID-Version-FileID
    static const std::regex nexus(R"(^(.*)-\d+-[\d\.\-]+-\d+$)");
    std::smatch m;
    if (std::regex_match(stem, m, nexus) && m.size() > 1)
    {
        return m[1].str();
    }
    return stem;
}

static std::string find_existing_fomod_json(const fs::path& fomod_output_dir,
                                            const std::string& mod_name,
                                            const std::string& archive_filename)
{
    if (fomod_output_dir.empty() || !fs::is_directory(fomod_output_dir))
    {
        return {};
    }

    const std::string archive_stem = fs::path(archive_filename).stem().string();
    const std::string archive_base = strip_nexus_suffix(archive_stem);
    const std::string mod_name_lower = mo2core::to_lower(mod_name);
    const std::string archive_stem_lower = mo2core::to_lower(archive_stem);
    const std::string archive_base_lower = mo2core::to_lower(archive_base);

    // Exact checks first.
    const fs::path exact_mod = fomod_output_dir / (mod_name + ".json");
    if (fs::exists(exact_mod))
        return exact_mod.string();
    const fs::path exact_archive = fomod_output_dir / (archive_stem + ".json");
    if (fs::exists(exact_archive))
        return exact_archive.string();
    const fs::path exact_archive_base = fomod_output_dir / (archive_base + ".json");
    if (fs::exists(exact_archive_base))
        return exact_archive_base.string();

    // Fuzzy: longest-stem-first where output stem is a prefix of mod/archive key.
    std::vector<fs::path> candidates;
    for (const auto& entry : fs::directory_iterator(fomod_output_dir))
    {
        if (entry.is_regular_file() && entry.path().extension() == ".json")
        {
            candidates.push_back(entry.path());
        }
    }
    std::sort(candidates.begin(),
              candidates.end(),
              [](const fs::path& a, const fs::path& b)
              { return a.stem().string().size() > b.stem().string().size(); });

    for (const auto& candidate : candidates)
    {
        const std::string stem_lower = mo2core::to_lower(candidate.stem().string());
        if (mod_name_lower.starts_with(stem_lower) || archive_stem_lower.starts_with(stem_lower) ||
            archive_base_lower.starts_with(stem_lower))
        {
            return candidate.string();
        }
    }

    return {};
}

crow::response InstallationController::handle_upload(const crow::request& req)
{
    auto& logger = mo2core::Logger::instance();

    try
    {
        crow::multipart::message msg(req);

        // Save uploaded file
        auto uploaded = MultipartHandler::save_uploaded_file(msg, "file");
        if (uploaded.temp_path.empty())
        {
            return json_response(400, {{"error", "No file uploaded or file is empty"}});
        }

        logger.log(std::format("[install] File saved to {} (extension: {})",
                               uploaded.temp_path,
                               uploaded.original_extension));

        // Get optional fields
        auto mod_name = MultipartHandler::get_part_value(msg, "modName");
        auto mod_path = MultipartHandler::get_part_value(msg, "modPath");
        auto fomod_json = MultipartHandler::get_part_value(msg, "fomodJson");

        // Handle FOMOD JSON - write to temp file before launching thread
        std::string json_path;
        bool json_is_temp = false;
        if (!fomod_json.empty())
        {
            logger.log(std::format("[install] FOMOD JSON provided ({} chars)", fomod_json.size()));
            json_path = fs::path(uploaded.temp_path).replace_extension(".json").string();
            std::ofstream ofs(json_path);
            ofs << fomod_json;
            ofs.close();
            json_is_temp = true;
            logger.log(std::format("[install] FOMOD JSON saved to {}", json_path));
        }

        // Generate mod name from filename if not provided
        if (mod_name.empty())
        {
            mod_name = fs::path(uploaded.filename).stem().string();
            static const std::regex kLeadingDigitsRegex("^\\d+[-_]");
            mod_name = std::regex_replace(mod_name, kLeadingDigitsRegex, "");
        }

        // If no JSON was uploaded, try to resolve an existing one from Salma FOMOD output.
        if (json_path.empty())
        {
            const auto fomod_output_dir = ConfigService::instance().fomod_output_dir();
            auto resolved = find_existing_fomod_json(fomod_output_dir, mod_name, uploaded.filename);
            if (!resolved.empty())
            {
                json_path = resolved;
                logger.log(std::format(R"([install] Using existing FOMOD JSON: "{}")", json_path));
            }
            else
            {
                logger.log("[install] No existing FOMOD JSON match found in Salma FOMODs Output");
            }
        }

        // Generate mod path if not provided
        if (mod_path.empty())
        {
            auto mods_dir = ConfigService::instance().mo2_mods_path();
            if (mods_dir.empty())
            {
                return json_response(
                    400, {{"error", "No modPath provided and MO2 mods path is not configured"}});
            }
            mod_path = (fs::path(mods_dir) / mod_name).string();
        }

        // Check if already busy
        if (install_running_.exchange(true))
        {
            return json_response(409, {{"error", "An installation is already running"}});
        }

        {
            std::lock_guard<std::mutex> lock(install_mutex_);
            install_has_result_ = false;
            install_last_error_.clear();
        }

        // Capture all values by copy - request object is invalid after return
        std::string temp_path = uploaded.temp_path;
        std::thread(
            [this, temp_path, mod_path, json_path, mod_name, json_is_temp]()
            {
#ifdef _WIN32
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
#endif
                auto& log = mo2core::Logger::instance();
                try
                {
                    log.log(std::format("[install] Starting installation to {}", mod_path));

                    mo2core::InstallationService service;
                    auto result = service.install_mod(temp_path, mod_path, json_path);

                    log.log(std::format("[install] Installation completed. Mod installed to {}",
                                        result));

                    std::lock_guard<std::mutex> lock(install_mutex_);
                    install_has_result_ = true;
                    install_last_success_ = true;
                    install_result_mod_path_ = result;
                    install_result_mod_name_ = mod_name;
                    install_last_error_.clear();
                }
                catch (const std::exception& ex)
                {
                    log.log_error(std::format("[install] Error processing archive: {}", ex.what()));
                    std::lock_guard<std::mutex> lock(install_mutex_);
                    install_has_result_ = true;
                    install_last_success_ = false;
                    install_last_error_ = ex.what();
                }

                // Cleanup temp files
                try
                {
                    fs::remove(temp_path);
                    if (json_is_temp && !json_path.empty() && fs::exists(json_path))
                        fs::remove(json_path);
                }
                catch (const std::exception& ex)
                {
                    mo2core::Logger::instance().log_warning(
                        std::format("[install] Temp file cleanup failed: {}", ex.what()));
                }

                install_running_.store(false);
            })
            .detach();

        return json_response(200, {{"started", true}, {"modName", mod_name}});
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[install] Error processing upload: {}", ex.what()));
        return json_response(500, {{"error", ex.what()}});
    }
}

crow::response InstallationController::handle_install(const crow::request& req)
{
    auto& logger = mo2core::Logger::instance();

    try
    {
        auto body = json::parse(req.body);
        auto archive_path = body.value("archivePath", "");
        auto mod_path_val = body.value("modPath", "");
        auto json_path = body.value("jsonPath", "");

        if (archive_path.empty() || mod_path_val.empty())
        {
            return json_response(400, {{"error", "ArchivePath and ModPath are required"}});
        }

        if (install_running_.exchange(true))
        {
            return json_response(409, {{"error", "An installation is already running"}});
        }

        {
            std::lock_guard<std::mutex> lock(install_mutex_);
            install_has_result_ = false;
            install_last_error_.clear();
        }

        std::thread(
            [this, archive_path, mod_path_val, json_path]()
            {
#ifdef _WIN32
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
#endif
                auto& log = mo2core::Logger::instance();
                try
                {
                    log.log(std::format("[install] Starting installation to {}", mod_path_val));

                    mo2core::InstallationService service;
                    auto result = service.install_mod(archive_path, mod_path_val, json_path);

                    log.log(std::format("[install] Installation completed. Mod installed to {}",
                                        result));

                    std::lock_guard<std::mutex> lock(install_mutex_);
                    install_has_result_ = true;
                    install_last_success_ = true;
                    install_result_mod_path_ = result;
                    install_last_error_.clear();
                }
                catch (const std::exception& ex)
                {
                    log.log_error(std::format("[install] Error installing mod: {}", ex.what()));
                    std::lock_guard<std::mutex> lock(install_mutex_);
                    install_has_result_ = true;
                    install_last_success_ = false;
                    install_last_error_ = ex.what();
                }

                install_running_.store(false);
            })
            .detach();

        return json_response(200, {{"started", true}});
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[install] Error installing mod: {}", ex.what()));
        return json_response(500, {{"error", ex.what()}});
    }
}

crow::response InstallationController::handle_status(const std::string& /*job_id*/)
{
    json result = {{"running", install_running_.load()}};
    std::lock_guard<std::mutex> lock(install_mutex_);
    if (install_has_result_)
    {
        result["success"] = install_last_success_;
        if (install_last_success_)
        {
            result["modPath"] = install_result_mod_path_;
            result["modName"] = install_result_mod_name_;
        }
        if (!install_last_error_.empty())
            result["error"] = install_last_error_;
    }
    return json_response(200, result);
}

}  // namespace mo2server

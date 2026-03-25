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
#include <vector>

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

    // Require a minimum stem length of 5 and at least 50% of the target length
    // to prevent short stems like "a.json" from matching unrelated mods.
    for (const auto& candidate : candidates)
    {
        const std::string stem_lower = mo2core::to_lower(candidate.stem().string());
        if (stem_lower.size() < 5)
        {
            continue;
        }

        auto is_plausible_match = [&](const std::string& target) -> bool
        {
            if (!target.starts_with(stem_lower))
                return false;
            if (stem_lower.size() * 2 < target.size())
                return false;
            if (stem_lower.size() == target.size())
                return true;
            // After the stem, the target must have a separator (not a word character)
            char next = target[stem_lower.size()];
            return next == '-' || next == '_' || next == ' ' || next == '.';
        };

        if (is_plausible_match(mod_name_lower) || is_plausible_match(archive_stem_lower) ||
            is_plausible_match(archive_base_lower))
        {
            return candidate.string();
        }
    }

    return {};
}

std::optional<InstallationController::UploadContext>
InstallationController::parse_and_validate_upload(const crow::request& req,
                                                  crow::response& error_out)
{
    auto& logger = mo2core::Logger::instance();

    // Enforce upload body-size limit. Crow's stream_threshold only controls
    // buffering behavior, not maximum allowed size, so we check explicitly.
    static constexpr size_t kMaxUploadBytes = 512ULL * 1024 * 1024;
    if (req.body.size() > kMaxUploadBytes)
    {
        error_out = json_response(413, {{"error", "Upload exceeds 512 MiB limit"}});
        return std::nullopt;
    }

    crow::multipart::message msg(req);

    // Save uploaded file
    auto uploaded = MultipartHandler::save_uploaded_file(msg, "file");
    if (uploaded.temp_path.empty())
    {
        error_out = json_response(400, {{"error", "No file uploaded or file is empty"}});
        return std::nullopt;
    }

    UploadContext ctx;
    ctx.temp_path = uploaded.temp_path;
    ctx.filename = uploaded.filename;

    logger.log(std::format("[install] File saved to {} (extension: {})",
                           uploaded.temp_path,
                           uploaded.original_extension));

    // Get optional fields
    auto mod_name = MultipartHandler::get_part_value(msg, "modName");
    auto mod_path = MultipartHandler::get_part_value(msg, "modPath");
    auto fomod_json = MultipartHandler::get_part_value(msg, "fomodJson");

    // Handle FOMOD JSON - write to temp file before launching thread
    if (!fomod_json.empty())
    {
        logger.log(std::format("[install] FOMOD JSON provided ({} chars)", fomod_json.size()));
        std::string json_path = fs::path(uploaded.temp_path).replace_extension(".json").string();
        std::ofstream ofs(json_path);
        ofs << fomod_json;
        ofs.close();
        if (ofs.fail())
        {
            logger.log_warning(
                std::format("[install] Failed to write temp FOMOD JSON to {}", json_path));
        }
        else
        {
            ctx.json_path = json_path;
            ctx.json_is_temp = true;
            logger.log(std::format("[install] FOMOD JSON saved to {}", json_path));
        }
    }

    // Generate mod name from filename if not provided
    if (mod_name.empty())
    {
        mod_name = fs::path(uploaded.filename).stem().string();
        static const std::regex kLeadingDigitsRegex("^\\d+[-_]");
        mod_name = std::regex_replace(mod_name, kLeadingDigitsRegex, "");
    }
    ctx.mod_name = mod_name;

    // If no JSON was uploaded, try to resolve an existing one from Salma FOMOD output.
    if (ctx.json_path.empty())
    {
        const auto fomod_output_dir = ConfigService::instance().fomod_output_dir();
        auto resolved = find_existing_fomod_json(fomod_output_dir, mod_name, uploaded.filename);
        if (!resolved.empty())
        {
            ctx.json_path = resolved;
            logger.log(std::format(R"([install] Using existing FOMOD JSON: "{}")", ctx.json_path));
        }
        else
        {
            logger.log("[install] No existing FOMOD JSON match found in Salma FOMODs Output");
        }
    }

    if (!mod_path.empty())
    {
        auto mods_dir_check = mo2server::ConfigService::instance().mo2_mods_path();
        if (mods_dir_check.empty())
        {
            error_out = json_response(
                400, {{"error", "Cannot validate modPath: MO2 mods directory is not configured"}});
            return std::nullopt;
        }
        if (!mo2core::is_inside(fs::path(mods_dir_check), fs::path(mod_path)))
        {
            error_out = json_response(
                400, {{"error", "modPath must be inside the configured MO2 mods directory"}});
            return std::nullopt;
        }
    }

    // Generate mod path if not provided
    if (mod_path.empty())
    {
        auto mods_dir = ConfigService::instance().mo2_mods_path();
        if (mods_dir.empty())
        {
            error_out = json_response(
                400, {{"error", "No modPath provided and MO2 mods path is not configured"}});
            return std::nullopt;
        }
        mod_path = (fs::path(mods_dir) / mod_name).string();
    }
    ctx.mod_path = mod_path;

    return ctx;
}

crow::response InstallationController::handle_upload(const crow::request& req)
{
    auto& logger = mo2core::Logger::instance();

    std::string temp_path_cleanup;
    std::string json_path_cleanup;
    bool json_temp_cleanup = false;

    try
    {
        crow::response error_out;
        auto ctx = parse_and_validate_upload(req, error_out);
        if (!ctx)
        {
            return error_out;
        }

        // Track temp files for cleanup in case the job cannot start or an exception is thrown
        temp_path_cleanup = ctx->temp_path;
        if (ctx->json_is_temp)
        {
            json_path_cleanup = ctx->json_path;
            json_temp_cleanup = true;
        }

        // Capture context fields by value for the background thread
        std::string temp_path = ctx->temp_path;
        std::string mod_path = ctx->mod_path;
        std::string json_path = ctx->json_path;
        std::string mod_name = ctx->mod_name;
        bool json_is_temp = ctx->json_is_temp;

        if (!job_.try_start(
                [temp_path, mod_path, json_path, mod_name, json_is_temp]() -> InstallJobResult
                {
                    auto& log = mo2core::Logger::instance();
                    InstallJobResult job_result;
                    try
                    {
                        log.log(std::format("[install] Starting installation to {}", mod_path));

                        mo2core::InstallationService service;
                        auto result = service.install_mod(temp_path, mod_path, json_path);

                        log.log(std::format("[install] Installation completed. Mod installed to {}",
                                            result));

                        job_result.success = true;
                        job_result.mod_path = result;
                        job_result.mod_name = mod_name;
                    }
                    catch (const std::exception& ex)
                    {
                        log.log_error(
                            std::format("[install] Error processing archive: {}", ex.what()));
                        job_result.success = false;
                        job_result.error = ex.what();
                    }

                    // Cleanup temp files
                    try
                    {
                        fs::remove(temp_path);
                        if (json_is_temp && !json_path.empty() && fs::exists(json_path))
                        {
                            fs::remove(json_path);
                        }
                    }
                    catch (const std::exception& ex)
                    {
                        mo2core::Logger::instance().log_warning(
                            std::format("[install] Temp file cleanup failed: {}", ex.what()));
                    }

                    return job_result;
                }))
        {
            // Job could not start -- clean up temp files that won't be used
            try
            {
                if (!temp_path_cleanup.empty())
                {
                    fs::remove(temp_path_cleanup);
                }
                if (json_temp_cleanup && !json_path_cleanup.empty())
                {
                    fs::remove(json_path_cleanup);
                }
            }
            catch (...)
            {
                logger.log_warning("[install] Failed to clean up temp files after 409");
            }
            return json_response(409, {{"error", "An installation is already running"}});
        }

        // Job started -- the lambda now owns temp file cleanup.
        // Clear caller's copies so the outer catch block won't double-delete.
        temp_path_cleanup.clear();
        json_path_cleanup.clear();
        json_temp_cleanup = false;

        return json_response(200, {{"started", true}, {"modName", ctx->mod_name}});
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[install] Error processing upload: {}", ex.what()));
        try
        {
            if (!temp_path_cleanup.empty())
                fs::remove(temp_path_cleanup);
            if (json_temp_cleanup && !json_path_cleanup.empty())
                fs::remove(json_path_cleanup);
        }
        catch (...)
        {
        }
        return json_response(500, {{"error", "Internal error during upload"}});
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

        if (!fs::exists(archive_path))
        {
            return json_response(400, {{"error", "Archive path does not exist"}});
        }

        // Validate archive_path is in an expected directory
        {
            auto mods_path = mo2server::ConfigService::instance().mo2_mods_path();
            const char* downloads_env = std::getenv("SALMA_DOWNLOADS_PATH");
            std::string downloads_path = (downloads_env && *downloads_env) ? downloads_env : "";

            bool archive_contained = false;
            if (!mods_path.empty() &&
                mo2core::is_inside(fs::path(mods_path), fs::path(archive_path)))
            {
                archive_contained = true;
            }
            if (!downloads_path.empty() &&
                mo2core::is_inside(fs::path(downloads_path), fs::path(archive_path)))
            {
                archive_contained = true;
            }

            if (!mods_path.empty() || !downloads_path.empty())
            {
                if (!archive_contained)
                {
                    return json_response(400,
                                         {{"error",
                                           "archivePath must be inside the configured mods or "
                                           "downloads directory"}});
                }
            }
            else
            {
                logger.log_warning(
                    "[install] No mods/downloads path configured; "
                    "archive_path containment check skipped");
            }
        }

        auto configured_mods = mo2server::ConfigService::instance().mo2_mods_path();
        if (configured_mods.empty())
        {
            return json_response(
                400, {{"error", "Cannot validate modPath: MO2 mods directory is not configured"}});
        }
        if (!mo2core::is_inside(fs::path(configured_mods), fs::path(mod_path_val)))
        {
            return json_response(
                400, {{"error", "modPath must be inside the configured MO2 mods directory"}});
        }

        // Validate jsonPath containment to prevent arbitrary file reads
        if (!json_path.empty())
        {
            if (!fs::exists(json_path))
            {
                return json_response(400, {{"error", "jsonPath does not exist"}});
            }

            auto mods_path_j = mo2server::ConfigService::instance().mo2_mods_path();
            const char* downloads_env_j = std::getenv("SALMA_DOWNLOADS_PATH");
            std::string downloads_path_j =
                (downloads_env_j && *downloads_env_j) ? downloads_env_j : "";
            auto fomod_output = mo2server::ConfigService::instance().fomod_output_dir();

            bool json_contained = false;
            if (!mods_path_j.empty() &&
                mo2core::is_inside(fs::path(mods_path_j), fs::path(json_path)))
                json_contained = true;
            if (!downloads_path_j.empty() &&
                mo2core::is_inside(fs::path(downloads_path_j), fs::path(json_path)))
                json_contained = true;
            if (!fomod_output.empty() && mo2core::is_inside(fomod_output, fs::path(json_path)))
                json_contained = true;
            // Allow jsonPath next to the archive
            if (!archive_path.empty())
            {
                auto archive_parent = fs::path(archive_path).parent_path();
                if (!archive_parent.empty() &&
                    mo2core::is_inside(archive_parent, fs::path(json_path)))
                    json_contained = true;
            }

            if (!json_contained &&
                (!mods_path_j.empty() || !downloads_path_j.empty() || !fomod_output.empty()))
            {
                return json_response(
                    400,
                    {{"error",
                      "jsonPath must be inside the configured mods, downloads, FOMOD output, "
                      "or archive directory"}});
            }
        }

        if (!job_.try_start(
                [archive_path, mod_path_val, json_path]() -> InstallJobResult
                {
                    auto& log = mo2core::Logger::instance();
                    InstallJobResult job_result;
                    try
                    {
                        log.log(std::format("[install] Starting installation to {}", mod_path_val));

                        mo2core::InstallationService service;
                        auto result = service.install_mod(archive_path, mod_path_val, json_path);

                        log.log(std::format("[install] Installation completed. Mod installed to {}",
                                            result));

                        job_result.success = true;
                        job_result.mod_path = result;
                    }
                    catch (const std::exception& ex)
                    {
                        log.log_error(std::format("[install] Error installing mod: {}", ex.what()));
                        job_result.success = false;
                        job_result.error = ex.what();
                    }

                    return job_result;
                }))
        {
            return json_response(409, {{"error", "An installation is already running"}});
        }

        return json_response(200, {{"started", true}});
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[install] Error installing mod: {}", ex.what()));
        return json_response(500, {{"error", "Internal error during installation"}});
    }
}

crow::response InstallationController::handle_status(const std::string& job_id)
{
    if (job_id != "current")
    {
        mo2core::Logger::instance().log_warning(std::format(
            "[install] handle_status: job_id '{}' ignored, only 'current' is supported", job_id));
    }

    // Read running state inside read_result's mutex-held callback so that
    // it is consistent with has_result (BackgroundJob sets running_=false
    // under the same mutex after storing the result).
    json result = job_.read_result(
        [this](bool has_result, const InstallJobResult* r, const std::string& error) -> json
        {
            json j = {{"running", job_.is_running()}};
            if (has_result && r)
            {
                // If BackgroundJob caught an exception, report it as a failed install.
                if (!error.empty())
                {
                    j["success"] = false;
                    j["error"] = error;
                }
                else
                {
                    j["success"] = r->success;
                    if (r->success)
                    {
                        j["modPath"] = r->mod_path;
                        if (!r->mod_name.empty())
                        {
                            j["modName"] = r->mod_name;
                        }
                    }
                    if (!r->error.empty())
                    {
                        j["error"] = r->error;
                    }
                }
            }
            return j;
        });

    return json_response(200, result);
}

}  // namespace mo2server

#include "Mo2Controller.h"
#include "Mo2Helpers.h"

#include "ConfigService.h"
#include "Logger.h"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// ---------------------------------------------------------------------------
// GET /api/config
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_config()
{
    auto& cfg = ConfigService::instance();
    auto mods = cfg.mo2_mods_path();
    auto fomod_dir = cfg.fomod_output_dir();

    json j = {{"mo2ModsPath", mods},
              {"fomodOutputDir", fomod_dir.string()},
              {"mo2ModsPathValid", !mods.empty() && fs::is_directory(mods)}};
    return json_response(200, j);
}

// ---------------------------------------------------------------------------
// PUT /api/config
// ---------------------------------------------------------------------------

crow::response Mo2Controller::put_config(const crow::request& req)
{
    try
    {
        auto body = json::parse(req.body);
        auto& cfg = ConfigService::instance();

        if (body.contains("mo2ModsPath"))
        {
            auto mods_path = body["mo2ModsPath"].get<std::string>();
            if (mods_path.empty())
                return json_response(400, {{"error", "mo2ModsPath must not be empty"}});
            {
                // Check for ".." as a discrete path segment rather than a substring
                // to avoid rejecting legitimate directories containing ".." (e.g. "My..Mod").
                auto p = fs::path(mods_path);
                for (const auto& seg : p)
                {
                    if (seg == "..")
                        return json_response(
                            400, {{"error", "mo2ModsPath must not contain '..' segments"}});
                }
            }
            if (!fs::is_directory(mods_path))
                return json_response(
                    400, {{"error", "mo2ModsPath does not exist or is not a directory"}});

            // Transactional: stage in memory, persist to disk, revert on failure.
            // Without this the in-memory state can diverge from disk when the
            // save fails (disk full, permissions), and the next restart silently
            // reverts changes the user already saw applied.
            if (!cfg.apply_mo2_mods_path(mods_path))
            {
                return json_response(500, {{"error", "Failed to persist configuration to disk"}});
            }
        }
        else
        {
            // No mods path in body -- caller is using PUT as a "re-save" of
            // the current state (or sending an unrelated field). Best-effort save.
            if (!cfg.save())
            {
                return json_response(500, {{"error", "Failed to persist configuration to disk"}});
            }
        }
        return get_config();
    }
    catch (const std::exception& ex)
    {
        mo2core::Logger::instance().log_error(
            std::format("[server] Config update failed: {}", ex.what()));
        return json_response(400, {{"error", "Invalid request"}});
    }
}

}  // namespace mo2server

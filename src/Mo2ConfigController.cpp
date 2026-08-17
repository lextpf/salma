#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "ConfigService.hpp"
#include "Logger.hpp"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

// Mo2ConfigController - the /api/config read and write.
//
// mo2ModsPath is the only key the server persists; everything else in the
// response is derived from it at read time. A successful PUT returns the same
// body get_config() would, so the caller never has to re-read.
//
// PUT accepts a body without mo2ModsPath and treats it as a re-save of the
// current state, which is why an unrelated field is not an error.

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
                // Match ".." as a whole path segment, not as a substring. A
                // substring test would reject a real directory such as
                // "My..Mod".
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

            // apply_mo2_mods_path stages the value, saves, and reverts if the
            // save fails. A plain assign-then-save would leave memory and disk
            // disagreeing after a full disk or a permission error, and the next
            // restart would silently undo a change the user saw applied.
            if (!cfg.apply_mo2_mods_path(mods_path))
            {
                return json_response(500, {{"error", "Failed to persist configuration to disk"}});
            }
        }
        else
        {
            // No mods path in the body: treat the request as a re-save of the
            // current state.
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

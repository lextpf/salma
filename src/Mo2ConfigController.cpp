#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "ConfigService.hpp"
#include "Logger.hpp"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

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
                // reject dot-dot components without rejecting names such as My..Mod.
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

            // use the transactional setter so a failed save restores memory.
            if (!cfg.apply_mo2_mods_path(mods_path))
            {
                return json_response(500, {{"error", "Failed to persist configuration to disk"}});
            }
        }
        else
        {
            // an absent key requests a save of the current value.
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

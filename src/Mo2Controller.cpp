#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "ConfigService.hpp"
#include "Logger.hpp"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

// Mo2Controller - the class's lifetime and GET /api/mo2/status.
//
// One class, several translation units. Its handlers are split by concern:
// Mo2ConfigController.cpp, Mo2FomodController.cpp, Mo2LogController.cpp,
// Mo2PluginController.cpp and Mo2TestController.cpp. This file owns only the
// constructor, the destructor and the status read; shared helpers live in
// Mo2Helpers.
//
// get_status reports a snapshot the dashboard polls on a timer, so it answers
// from a 5-second cache. The scan job invalidates that cache when it finishes.

namespace mo2server
{

// ---------------------------------------------------------------------------
// Constructor / Destructor
// ---------------------------------------------------------------------------

Mo2Controller::Mo2Controller() = default;

Mo2Controller::~Mo2Controller()
{
    // Shut the jobs down here, at a known point in the controller's lifetime,
    // rather than leaving it to reverse-order member destruction. The header's
    // member order is therefore not load-bearing, and neither is the fact that
    // BackgroundJob's worker captures shared_ptr<State> by value, which already
    // lets the state outlive `*this` when a hung worker is detached. Today's
    // workers touch no Mo2Controller member; pinning the teardown order in code
    // keeps a future member reorder or capture change from reintroducing the
    // hazard.
    scan_job_.shutdown();
    plugin_action_job_.shutdown();

#ifdef _WIN32
    std::lock_guard<std::mutex> lock(test_mutex_);
    if (test_process_)
    {
        // A test child still running at shutdown would be orphaned once its
        // handle closes, so terminate it first. Best effort: log, terminate,
        // wait briefly, close.
        DWORD wait = WaitForSingleObject(test_process_, 0);
        if (wait == WAIT_TIMEOUT)
        {
            mo2core::Logger::instance().log_warning(
                "[server] Terminating in-flight test process during shutdown");
            TerminateProcess(test_process_, 1);
            WaitForSingleObject(test_process_, 5000);
        }
        CloseHandle(test_process_);
        test_process_ = nullptr;
        test_running_ = false;
    }
#endif
}

// ---------------------------------------------------------------------------
// GET /api/mo2/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_status()
{
    {
        std::lock_guard<std::mutex> lock(cache_mutex_);
        if (status_cache_.is_fresh(std::chrono::seconds(5)))
        {
            return json_response(200, status_cache_.data);
        }
    }

    auto& cfg = ConfigService::instance();
    auto fomod_dir = cfg.fomod_output_dir();
    auto mods_path = cfg.mo2_mods_path();
    auto deploy_path = resolve_deploy_path(mods_path);
    bool plugin_installed = plugin_installed_at(deploy_path);

    bool output_exists = !fomod_dir.empty() && fs::is_directory(fomod_dir);
    int json_count = 0;
    int mod_count = 0;

    if (output_exists)
    {
        for (auto& entry : fs::directory_iterator(fomod_dir))
        {
            if (entry.is_regular_file() && entry.path().extension() == ".json")
                ++json_count;
        }
    }

    // MO2 keeps one directory per mod directly under the mods path, so only the
    // top level is counted.
    if (!mods_path.empty() && fs::is_directory(mods_path))
    {
        for (auto& entry : fs::directory_iterator(mods_path))
        {
            if (entry.is_directory())
                ++mod_count;
        }
    }

    json j = {{"configured", !cfg.mo2_mods_path().empty()},
              {"outputFolderExists", output_exists},
              {"fomodOutputDir", fomod_dir.string()},
              {"jsonCount", json_count},
              {"modCount", mod_count},
              {"pluginInstalled", plugin_installed},
              {"pluginDeployPath", deploy_path.string()}};

    {
        std::lock_guard<std::mutex> lock(cache_mutex_);
        status_cache_.set(j);
    }

    return json_response(200, j);
}

}  // namespace mo2server

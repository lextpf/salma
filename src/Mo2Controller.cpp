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
// Constructor / Destructor
// ---------------------------------------------------------------------------

Mo2Controller::Mo2Controller() = default;

Mo2Controller::~Mo2Controller()
{
    // Tear down background work BEFORE other members destruct. The header's
    // member-declaration order is no longer load-bearing because:
    //   (a) BackgroundJob's worker captures shared_ptr<State> by value, so the
    //       state outlives `*this` even if we detach a hung worker, and
    //   (b) we explicitly shut the jobs down here, so the join (or detach)
    //       happens at a known point in the controller's lifetime instead of
    //       relying on reverse-order destruction to do it implicitly.
    // The current workers don't reference Mo2Controller members anyway, but
    // pinning the teardown order in code (not in a comment) prevents a future
    // member-reorder or capture change from silently introducing UB.
    scan_job_.shutdown();
    plugin_action_job_.shutdown();

#ifdef _WIN32
    std::lock_guard<std::mutex> lock(test_mutex_);
    if (test_process_)
    {
        // If a test child process is still running at shutdown, terminate it
        // rather than orphaning it with a closed handle. Best-effort: log,
        // ask Windows to terminate, wait briefly, then close the handle.
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

    // Count mod folders under the mods path (top-level directories)
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

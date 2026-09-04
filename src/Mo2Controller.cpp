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

Mo2Controller::Mo2Controller() = default;

Mo2Controller::~Mo2Controller()
{
    // stop workers before destroying state they could otherwise observe.
    scan_job_.shutdown();
    plugin_action_job_.shutdown();

#ifdef _WIN32
    std::lock_guard<std::mutex> lock(test_mutex_);
    if (test_process_)
    {
        // terminate the child before closing its last owned handle.
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

    // count only MO2's top-level mod directories.
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

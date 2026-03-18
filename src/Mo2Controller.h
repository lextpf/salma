#pragma once

#include <crow.h>
#include <atomic>
#include <mutex>
#include <string>
#include <thread>

#ifdef _WIN32
#include <windows.h>
#endif

namespace mo2server
{

/**
 * @class Mo2Controller
 * @brief REST endpoints for MO2 integration dashboard.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Mo2Controller
 *
 * Provides configuration management, FOMOD JSON browsing,
 * MO2 integration status, log tailing, and test execution.
 *
 * ## :material-api: Endpoints
 *
 * | Method | Route                         | Handler         |
 * |--------|-------------------------------|-----------------|
 * |    GET | `/api/config`                 | get_config      |
 * |    PUT | `/api/config`                 | put_config      |
 * |    GET | `/api/mo2/status`             | get_status      |
 * |    GET | `/api/mo2/fomods`             | list_fomods     |
 * |   POST | `/api/mo2/fomods/scan`        | scan_fomods     |
 * |    GET | `/api/mo2/fomods/scan/status` | get_scan_status |
 * |    GET | `/api/mo2/fomods/<name>`      | get_fomod       |
 * | DELETE | `/api/mo2/fomods/<name>`      | delete_fomod    |
 * |   POST | `/api/plugin/deploy`          | deploy_plugin              |
 * |   POST | `/api/plugin/purge`           | purge_plugin               |
 * |    GET | `/api/plugin/status`          | get_plugin_action_status   |
 * |    GET | `/api/logs`                   | get_logs        |
 * |    GET | `/api/logs/test`              | get_test_logs   |
 * |   POST | `/api/logs/clear`             | clear_logs      |
 * |   POST | `/api/logs/clear/test`        | clear_test_logs |
 * |   POST | `/api/test/run`               | run_tests       |
 * |    GET | `/api/test/status`            | get_test_status |
 *
 * ## :material-code-braces: Request / Response Shapes
 *
 * **Type key:** `"..."` = string, `bool` = boolean, `int` = integer.
 * All error responses use `{ "error": "..." }` with an appropriate HTTP status code.
 *
 * ### Configuration
 *
 * **`GET /api/config`**
 * - Response: `{ "mo2ModsPath", "fomodOutputDir", "mo2ModsPathValid": bool }`
 *
 * **`PUT /api/config`**
 * - Request: `{ "mo2ModsPath": "..." }`
 * - Response: *(same as GET /api/config)*
 *
 * ### MO2 Integration
 *
 * **`GET /api/mo2/status`**
 * - Response: `{ "configured": bool, "outputFolderExists": bool,`
 *   `"fomodOutputDir", "jsonCount": int, "modCount": int,`
 *   `"pluginInstalled": bool, "pluginDeployPath" }`
 *
 * **`GET /api/mo2/fomods`**
 * - Response: `[{ "name", "size": int, "modified": int, "stepCount": int }]`
 *
 * **`POST /api/mo2/fomods/scan`**
 * - Response: `{ "success": true, "running": true, "started": true }`
 * - Errors: 400 bad path, 409 busy, 500 mkdir
 *
 * **`GET /api/mo2/fomods/scan/status`**
 * - Response: `{ "running": bool, "success": bool,`
 *   `"totalModFolders": int, "archivesProcessed": int,`
 *   `"choicesInferred": int, "noFomod": int, "alreadyHadChoices": int,`
 *   `"noArchiveFound": int, "archiveMissing": int, "errors": int,`
 *   `"durationMs": int, "outputDir", "error"? }`
 * - Fields after `running` only present when scan has completed.
 *
 * **`GET /api/mo2/fomods/<name>`**
 * - Response: raw FOMOD JSON contents
 * - Errors: 403 traversal, 404 missing
 *
 * **`DELETE /api/mo2/fomods/<name>`**
 * - Response: `{ "success": true }`
 * - Errors: 403 traversal, 404 missing
 *
 * ### Plugin Management
 *
 * **`POST /api/plugin/deploy`** and **`POST /api/plugin/purge`**
 * - Response: `{ "started": true, "action": "deploy"|"purge" }`
 * - Errors: 404 script missing, 409 busy, 501 non-Windows
 *
 * **`GET /api/plugin/status`**
 * - Response: `{ "running": bool, "success": bool, "exitCode": int,`
 *   `"pluginInstalled": bool, "pluginDeployPath", "action", "error"? }`
 * - Fields after `running` only present when action has completed.
 *
 * ### Logs
 *
 * **`GET /api/logs`** and **`GET /api/logs/test`**
 * - Query: `?lines=N` (default 100, max 5000)
 * - Response: `{ "lines": ["..."] }`
 *
 * **`POST /api/logs/clear`** and **`POST /api/logs/clear/test`**
 * - Response: `{ "success": true }`
 *
 * ### Test Runner
 *
 * **`POST /api/test/run`**
 * - Request: `{ "args"?: "..." }`
 * - Response: `{ "running": true, "pid": int }`
 * - Errors: 404 test.py missing, 409 busy, 500 CreateProcess, 501 non-Windows
 *
 * **`GET /api/test/status`**
 * - Response: `{ "running": bool }` -- when finished includes `"exitCode": int`
 *
 * ## :material-sync: Async Scan Lifecycle
 *
 * `POST /api/mo2/fomods/scan` launches a background `std::thread`
 * (detached) that iterates every mod folder under the configured
 * MO2 mods path and infers FOMOD selections. The lifecycle is:
 *
 * 1. **Start** -- returns 200 immediately; sets `scan_running_` to true.
 *    Returns 409 if a scan is already running.
 * 2. **Poll** -- `GET /api/mo2/fomods/scan/status` returns `running`,
 *    and once finished, a full summary (success, counts, duration).
 * 3. **Completion** -- the thread stores results under `scan_mutex_`,
 *    sets `scan_has_result_` to true, and clears `scan_running_`.
 *
 * Test execution (`POST /api/test/run`) follows the same pattern
 * with `test_running_` / `test_mutex_`.
 *
 * **Contract drift:** The endpoint table above and the response shapes
 * are manually maintained. When adding, removing, or changing routes
 * in main.cpp and their handler implementations, update this doc block
 * to keep them in sync.
 *
 * @see ConfigService
 */
class Mo2Controller
{
public:
    Mo2Controller();
    ~Mo2Controller();
    Mo2Controller(const Mo2Controller&) = delete;
    Mo2Controller& operator=(const Mo2Controller&) = delete;

    crow::response get_config();
    crow::response put_config(const crow::request& req);
    crow::response get_status();
    crow::response list_fomods();
    crow::response scan_fomods();
    crow::response get_scan_status();
    crow::response get_fomod(const std::string& name);
    crow::response delete_fomod(const std::string& name);
    crow::response deploy_plugin();
    crow::response purge_plugin();
    crow::response get_plugin_action_status();
    crow::response get_logs(const crow::request& req);
    crow::response get_test_logs(const crow::request& req);
    crow::response clear_logs();
    crow::response clear_test_logs();
    crow::response run_tests(const crow::request& req);
    crow::response get_test_status();

private:
    // -- Test runner state --
    std::mutex test_mutex_;                  // guards test_process_
    std::atomic<bool> test_running_{false};  // true while test.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test.py
#endif

    // -- FOMOD scan state (written by background thread, read by status endpoint) --
    std::atomic<bool> scan_running_{false};  // true while scan thread is active
    std::mutex scan_mutex_;                  // guards all scan_ fields below
    bool scan_has_result_{false};            // true after first scan completes
    bool scan_last_success_{false};          // overall success of last scan
    int scan_total_mod_folders_{0};          // total mod folders examined
    int scan_archives_processed_{0};         // archives successfully processed
    int scan_choices_inferred_{0};           // mods where selections were inferred
    int scan_no_fomod_{0};                   // mods with no ModuleConfig.xml
    int scan_already_had_choices_{0};        // mods that already had a choices JSON
    int scan_no_archive_found_{0};           // mods with no matching archive on disk
    int scan_archive_missing_{0};            // archives referenced but not found
    int scan_errors_{0};                     // mods that failed with an exception
    long long scan_duration_ms_{0};          // wall-clock duration of last scan
    std::string scan_output_dir_;            // directory where JSON results were written
    std::string scan_last_error_;            // error message if scan failed
    std::thread scan_thread_;                // background scan thread (joined in destructor)

    // -- Plugin deploy/purge state --
    std::mutex plugin_action_mutex_;                  // guards all plugin_action_ fields below
    std::atomic<bool> plugin_action_running_{false};  // true while deploy/purge script runs
    bool plugin_action_has_result_{false};            // true after first action completes
    bool plugin_action_last_success_{false};          // whether last action succeeded
    int plugin_action_exit_code_{0};                  // exit code of the PowerShell script
    bool plugin_action_plugin_installed_{false};      // whether the plugin DLL exists after action
    std::string plugin_action_deploy_path_;           // filesystem path the plugin was deployed to
    std::string plugin_action_last_error_;            // error message if action failed
    std::string plugin_action_type_;                  // "deploy" or "purge"
    std::thread plugin_action_thread_;                // background deploy/purge thread
};

}  // namespace mo2server

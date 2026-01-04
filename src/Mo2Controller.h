#pragma once

#include <crow.h>

#include "BackgroundJob.h"

#include <chrono>
#include <nlohmann/json.hpp>

#include <mutex>
#include <string>

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
 * `POST /api/mo2/fomods/scan` submits work to `scan_job_`
 * (`BackgroundJob<ScanResult>`) which spawns a joinable thread that
 * iterates every mod folder under the configured MO2 mods path and
 * infers FOMOD selections. The lifecycle is:
 *
 * 1. **Start** -- `scan_job_.try_start(work)` returns 200 immediately.
 *    Returns 409 if `scan_job_.is_running()` (a scan is already in progress).
 * 2. **Poll** -- `GET /api/mo2/fomods/scan/status` checks `scan_job_.is_running()`,
 *    and once finished, calls `scan_job_.read_result()` for the full summary
 *    (success, counts, duration).
 * 3. **Completion** -- `BackgroundJob` stores the `ScanResult` under its
 *    internal mutex and clears the running flag automatically.
 *
 * Plugin deploy/purge (`POST /api/plugin/deploy`, `/purge`) follows
 * the same `BackgroundJob` pattern via `plugin_action_job_`
 * (`BackgroundJob<PluginActionResult>`).
 *
 * Test execution (`POST /api/test/run`) does **not** use `BackgroundJob`;
 * it launches a Win32 process via `CreateProcessW` and tracks it with a
 * raw `HANDLE` (`test_process_`), polled by `get_test_status()` using
 * `WaitForSingleObject`. The `test_running_` flag and `test_mutex_` are
 * managed manually.
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

    crow::response run_plugin_action(const std::string& action);
    crow::response get_logs(const crow::request& req);
    crow::response get_test_logs(const crow::request& req);
    crow::response clear_logs();
    crow::response clear_test_logs();
    crow::response run_tests(const crow::request& req);
    crow::response get_test_status();

private:
    // -- Test runner state --
    std::mutex test_mutex_;     // guards test_process_
    bool test_running_{false};  // true while test.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test.py
#endif

    /// Result of a FOMOD scan job.
    struct ScanResult
    {
        bool success = false;
        int total_mod_folders = 0;
        int archives_processed = 0;
        int choices_inferred = 0;
        int no_fomod = 0;
        int already_had_choices = 0;
        int no_archive_found = 0;
        int archive_missing = 0;
        int errors = 0;
        long long duration_ms = 0;
        std::string output_dir;
    };

    /// Result of a plugin deploy/purge action.
    struct PluginActionResult
    {
        bool success = false;
        int exit_code = 0;
        bool plugin_installed = false;
        std::string deploy_path;
        std::string action;
    };

    // -- Response cache with TTL for frequently polled endpoints --
    struct CachedResponse
    {
        nlohmann::json data;
        std::chrono::steady_clock::time_point timestamp{};

        [[nodiscard]] bool is_fresh(std::chrono::seconds ttl) const
        {
            return timestamp.time_since_epoch().count() > 0 &&
                   (std::chrono::steady_clock::now() - timestamp) < ttl;
        }

        void set(nlohmann::json value)
        {
            data = std::move(value);
            timestamp = std::chrono::steady_clock::now();
        }

        void invalidate() { timestamp = {}; }
    };

    mutable std::mutex cache_mutex_;
    CachedResponse fomods_cache_;
    CachedResponse status_cache_;

    // -- Background jobs --
    // Declared AFTER cache_mutex_, fomods_cache_, and status_cache_ so that
    // C++ reverse-order destruction joins the background threads BEFORE
    // destroying the members they reference.
    BackgroundJob<ScanResult> scan_job_;
    BackgroundJob<PluginActionResult> plugin_action_job_;
};

}  // namespace mo2server

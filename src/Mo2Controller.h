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
 * - Query: `?lines=N` (default 100, max 5000), `?offset=B` (incremental tail)
 * - Response: `{ "lines": ["..."], "errors": int, "warnings": int,
 *   "passes": int, "nextOffset": int }`
 *
 * **`POST /api/logs/clear`** and **`POST /api/logs/clear/test`**
 * - Response: `{ "success": true }`
 *
 * ### Test Runner
 *
 * **`POST /api/test/run`**
 * - Request: `{ "args"?: "..." }`
 * - Response: `{ "running": true, "pid": int }`
 * - Errors: 404 test_all.py missing, 409 busy, 500 CreateProcess, 501 non-Windows
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
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * sequenceDiagram
 *     participant C as Client
 *     participant Ctrl as Mo2Controller
 *     participant Job as BackgroundJob
 *     C->>Ctrl: POST /api/mo2/fomods/scan
 *     Ctrl->>Job: try_start(work)
 *     alt running
 *       Job-->>Ctrl: false
 *       Ctrl-->>C: 409 already running
 *     else accepted
 *       Job-->>Ctrl: true
 *       Ctrl-->>C: 200 started
 *     end
 *     loop poll
 *       C->>Ctrl: GET /scan/status
 *       Ctrl->>Job: is_running() / read_result()
 *       Job-->>Ctrl: {running, ScanResult?}
 *       Ctrl-->>C: 200 status
 *     end
 * ```
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
 * ## :material-source-branch: Implementation File Split
 *
 * Handler bodies are split across multiple translation units to keep
 * each file under ~300 LOC and grouped by concern:
 *
 * | Translation unit            | Endpoints                                |
 * |-----------------------------|------------------------------------------|
 * | `Mo2Controller.cpp`         | constructors/destructors, shared state   |
 * | `Mo2ConfigController.cpp`   | `/api/config` (GET, PUT)                 |
 * | `Mo2FomodController.cpp`    | `/api/mo2/...` (status, fomods, scan)    |
 * | `Mo2LogController.cpp`      | `/api/logs/...`                          |
 * | `Mo2PluginController.cpp`   | `/api/plugin/...`                        |
 * | `Mo2TestController.cpp`     | `/api/test/...` plus Win32 process glue  |
 * | `Mo2Helpers.h/.cpp`         | shared helpers (json_response, paths)    |
 *
 * Add new handlers to the file that owns their endpoint group rather
 * than to `Mo2Controller.cpp`.
 *
 * ## :material-warehouse: Member-Order Invariant
 *
 * `cache_mutex_`, `fomods_cache_`, and `status_cache_` are declared
 * **before** `scan_job_` and `plugin_action_job_`. C++ destroys
 * members in reverse declaration order, so the BackgroundJob
 * destructors run -- and join their worker threads -- *before* the
 * cache mutex they may still touch is destroyed. Reordering these
 * fields would introduce a use-after-free at process shutdown.
 *
 * @see ConfigService, BackgroundJob
 */
class Mo2Controller
{
public:
    Mo2Controller();
    ~Mo2Controller();
    Mo2Controller(const Mo2Controller&) = delete;
    Mo2Controller& operator=(const Mo2Controller&) = delete;

    /**
     * @brief Returns current configuration (MO2 mods path, FOMOD output dir, path validity).
     * @return 200 with config object.
     */
    crow::response get_config();

    /**
     * @brief Validates and persists a new MO2 mods path, then returns the updated config.
     * @param req JSON body with `"mo2ModsPath"`. Rejects empty values, `..` segments, and
     * non-existent directories.
     * @return 200 with updated config, or 400/500 on validation/persistence failure.
     */
    crow::response put_config(const crow::request& req);

    /**
     * @brief Reports MO2 integration health: whether paths are configured, JSON/mod counts, and
     * plugin install state.
     * @return 200 with status object. Cached for 5 seconds.
     */
    crow::response get_status();

    /**
     * @brief Lists all FOMOD JSON files in the output directory with metadata (size, modified
     * time, step count).
     * @return 200 with JSON array of FOMOD summaries. Returns empty array if the output directory
     * is missing. Cached for 5 seconds.
     */
    crow::response list_fomods();

    /**
     * @brief Kicks off an async background scan that infers FOMOD selections for every mod under
     * the MO2 mods path.
     * @return 200 if the scan started, 400 if paths are misconfigured, 409 if a scan is already
     * running, 500 on mkdir failure.
     */
    crow::response scan_fomods();

    /**
     * @brief Polls the running/completed state of the last FOMOD scan job.
     * @return 200 with `running` flag; includes full scan summary (counts, duration) once
     * finished.
     */
    crow::response get_scan_status();

    /**
     * @brief Returns the parsed contents of a single FOMOD JSON file by mod name.
     * @param name URL-encoded mod name (without `.json` extension).
     * @return 200 with raw JSON content, 403 on path traversal, 404 if not found.
     */
    crow::response get_fomod(const std::string& name);

    /**
     * @brief Deletes a single FOMOD JSON file from the output directory.
     * @param name URL-encoded mod name (without `.json` extension).
     * @return 200 on success, 403 on path traversal, 404 if not found.
     */
    crow::response delete_fomod(const std::string& name);

    /**
     * @brief Runs the `deploy.bat` script in the background to install the Salma MO2 plugin.
     *
     * Both the deploy path and the configured MO2 mods path are scanned
     * for shell metacharacters before launch. The blocked set is:
     * `& | > < ^ % ! ( ) " ; ' \``. Any of these in either path
     * yields a 400 response, preventing command injection through
     * paths that flow into a `cmd.exe` child process.
     *
     * @return 200 if started, 400 if mods path unconfigured or contains shell metacharacters, 404
     * if script missing, 409 if busy, 501 on non-Windows.
     */
    crow::response deploy_plugin();

    /**
     * @brief Runs the `purge.bat` script in the background to uninstall the Salma MO2 plugin.
     *
     * Same metacharacter rejection rules as deploy_plugin().
     *
     * @return 200 if started, 400 if mods path unconfigured or contains shell metacharacters, 404
     * if script missing, 409 if busy, 501 on non-Windows.
     */
    crow::response purge_plugin();

    /**
     * @brief Polls the running/completed state of the last plugin deploy or purge action.
     * @return 200 with `running` flag; includes exit code, install state, and action name once
     * finished.
     */
    crow::response get_plugin_action_status();

    /**
     * @brief Shared implementation for deploy_plugin() and purge_plugin(). Validates paths, then
     * launches the corresponding batch script via BackgroundJob.
     * @param action Must be `"deploy"` or `"purge"`; any other value returns 400.
     * @return 200 if started, 400/404/409/501 on error (see deploy_plugin/purge_plugin docs).
     */
    crow::response run_plugin_action(const std::string& action);

    /**
     * @brief Returns the tail of `logs/salma.log`. Supports incremental reads via `offset` query
     * param.
     * @param req Query params: `?lines=N` (default 100, max 5000), `?offset=B` for incremental
     * mode.
     * @return 200 with `lines` array, keyword counts (errors/warnings/passes), and `nextOffset`
     * for polling.
     */
    crow::response get_logs(const crow::request& req);

    /**
     * @brief Returns the tail of `test.log`. Supports incremental reads via `offset` query param.
     * @param req Query params: `?lines=N` (default 100, max 5000), `?offset=B` for incremental
     * mode.
     * @return 200 with `lines` array, keyword counts (errors/warnings/passes), and `nextOffset`
     * for polling.
     */
    crow::response get_test_logs(const crow::request& req);

    /**
     * @brief Truncates `logs/salma.log` via Logger::clear_log() to coordinate with the persistent
     * file handle.
     * @return 200 on success, 500 if truncation fails.
     */
    crow::response clear_logs();

    /**
     * @brief Truncates `test.log` by opening it in trunc mode.
     * @return 200 on success, 500 if truncation fails.
     */
    crow::response clear_test_logs();

    /**
     * @brief Spawns `test_all.py` as a detached Win32 process. Only one test run may be active at a
     * time.
     *
     * Unlike `scan_fomods` / `deploy_plugin` / `purge_plugin`, this
     * endpoint does **not** use `BackgroundJob`: it starts a Win32
     * child via `CreateProcessW` and tracks it with a raw `HANDLE`
     * (`test_process_`), polled by `get_test_status` using
     * `WaitForSingleObject`. The `test_running_` flag and
     * `test_mutex_` are managed manually.
     *
     * The optional `"args"` string is matched against the whitelist
     * regex `^[a-zA-Z0-9 _\-\.]*$` (alphanumeric, space, underscore,
     * hyphen, dot). Quotes and path separators are deliberately
     * excluded to prevent argument-boundary escape and path traversal.
     * The `..` substring is rejected explicitly even within the
     * whitelist (since `.` is allowed individually).
     *
     * @param req Optional JSON body with `"args"` (whitelist-sanitized as above).
     * @return 200 with PID on success, 400 on bad args, 404 if test_all.py missing, 409 if already
     * running, 500 on CreateProcess failure, 501 on non-Windows.
     */
    crow::response run_tests(const crow::request& req);

    /**
     * @brief Checks whether the test_all.py process is still running. Cleans up the process handle
     * on completion.
     * @return 200 with `running` flag; includes `exitCode` once the process has finished.
     */
    crow::response get_test_status();

private:
    // -- Test runner state --
    std::mutex test_mutex_;     // guards test_process_
    bool test_running_{false};  // true while test_all.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test_all.py
#endif

    /** Result of a FOMOD scan job. */
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

    /** Result of a plugin deploy/purge action. */
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

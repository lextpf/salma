#pragma once

#include <crow.h>

#include "BackgroundJob.hpp"

#include <chrono>
#include <nlohmann/json.hpp>

#include <mutex>
#include <string>

namespace mo2server
{

/**
 * @class Mo2Controller
 * @brief Coordinates MO2 dashboard operations and background jobs.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Mo2Controller
 *
 * Status and FOMOD listings are cached for 5 seconds. Scan and plugin actions
 * use independent single-job slots. The test runner owns one child process.
 *
 * ### :material-state-machine: Shutdown and test lifecycle
 *
 * Shutdown requests cancellation before other state is destroyed. A cancelled
 * scan reports partial counters with success still set. The Win32 test child is
 * terminated at shutdown and given 5 seconds to exit.
 *
 * ```mermaid
 * stateDiagram-v2
 *     [*] --> idle
 *     idle --> running: run_tests spawns the child
 *     running --> running: poll before exit
 *     running --> idle: poll reaps the result
 *     running --> [*]: shutdown terminates and waits 5 seconds
 * ```
 *
 * @warning The scan worker captures this controller to invalidate its caches.
 *          If inference exceeds the shutdown grace period, the detached worker
 *          can access destroyed controller state. Cancellation is checked between mods.
 *
 * @see ConfigService, BackgroundJob
 */
class Mo2Controller
{
public:
    /**
     * @fn Mo2Controller::Mo2Controller()
     * @brief Initialize empty caches and idle background jobs.
     * @author Alex (<https://github.com/lextpf>)
     */
    Mo2Controller();
    /**
     * @fn Mo2Controller::~Mo2Controller()
     * @brief Request worker shutdown and close the owned test process.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Each background slot has a 10-second grace period. A live Windows test child is terminated
     * and given up to five seconds to exit.
     */
    ~Mo2Controller();
    Mo2Controller(const Mo2Controller&) = delete;
    Mo2Controller& operator=(const Mo2Controller&) = delete;

    /**
     * @fn crow::response Mo2Controller::get_config()
     * @brief Includes path validity and the derived output directory.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return HTTP 200 with paths and a path-validity flag.
     */
    crow::response get_config();

    /**
     * @fn crow::response Mo2Controller::put_config(const crow::request&)
     * @brief Rejects missing directories before changing live state.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Empty values, dot-dot components, and missing directories are rejected.
     * An absent key saves the current value again.
     *
     * @param req JSON body with optional `mo2ModsPath`.
     * @return HTTP 200, 400 for invalid input, or 500 when persistence fails.
     */
    crow::response put_config(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_status()
     * @brief Serves five-second cached counts of top-level entries.
     * @author Alex (<https://github.com/lextpf>)
     *
     * JSON files and top-level mod directories are counted without recursion.
     *
     * @return HTTP 200 with a snapshot cached for 5 seconds.
     */
    crow::response get_status();

    /**
     * @fn crow::response Mo2Controller::list_fomods()
     * @brief Bounds repeat filesystem work with a five-second cache.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Modified times use Unix epoch milliseconds. Order is filesystem-defined.
     * SAX counting stops after the steps array. A rejected parse can leave partial
     * counts; only caught exceptions set `parseError`. This is not JSON validation.
     *
     * @return HTTP 200 with a top-level listing cached for 5 seconds.
     */
    crow::response list_fomods();

    /**
     * @fn crow::response Mo2Controller::scan_fomods()
     * @brief Rejects overlap and preserves existing selection files.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The request creates the output directory. The worker skips existing selection files,
     * writes inferred files, and invalidates both response caches at completion.
     *
     * ### :material-transit-connection-variant: Scan flow
     *
     * ```mermaid
     * flowchart TD
     *     mod --> existing{choices exist?}
     *     existing -- yes --> skip[skip]
     *     existing -- no --> archive{archive resolves?}
     *     archive -- no --> skip
     *     archive -- yes --> infer[infer selections]
     *     infer -->|no steps| none[record no FOMOD]
     *     infer -->|steps| write[write temp and rename]
     * ```
     *
     * @return HTTP 200 after start, 400 for invalid paths, 409 when busy, or 500
     *         when the output directory cannot be created.
     */
    crow::response scan_fomods();

    /**
     * @fn crow::response Mo2Controller::get_scan_status()
     * @brief Permits a transient mixed running and completed snapshot.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A cancelled shutdown scan reports successful partial counters.
     *
     * @return HTTP 200 with running state and the last completed summary.
     */
    crow::response get_scan_status();

    /**
     * @fn crow::response Mo2Controller::get_fomod(const std::string&)
     * @brief Enforces decoded-path containment before parsing.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The name is URL-decoded and checked for containment. The response is
     * compact re-serialized JSON, not the original bytes.
     *
     * @param name URL-encoded mod name without the `.json` extension.
     * @return HTTP 200, 403 for traversal, 404 when missing, or 500 on read failure.
     */
    crow::response get_fomod(const std::string& name);

    /**
     * @fn crow::response Mo2Controller::delete_fomod(const std::string&)
     * @brief Enforces decoded-path containment before permanent removal.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Deletion is permanent. The listing cache can retain the entry for 5 seconds.
     *
     * @param name URL-encoded mod name without the `.json` extension.
     * @return HTTP 200, 403 for traversal, 404 when missing, or 500 on failure.
     */
    crow::response delete_fomod(const std::string& name);

    /**
     * @fn crow::response Mo2Controller::deploy_plugin()
     * @brief Runs the fixed deploy script through the single action slot.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Deploy and mods paths reject shell metacharacters before they enter the
     * batch environment. The script path is derived from the executable directory.
     *
     * @return HTTP 200 after start, 400 for unsafe paths, 404 when the script is
     *         missing, 409 when busy, or 501 outside Windows.
     */
    crow::response deploy_plugin();

    /**
     * @fn crow::response Mo2Controller::purge_plugin()
     * @brief Runs the fixed purge script through the single action slot.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return HTTP 200 after start, with the same failures as `deploy_plugin`.
     * @warning Purge permanently removes plugin files and the complete
     *          `Salma FOMODs Output` directory without confirmation.
     */
    crow::response purge_plugin();

    /**
     * @fn crow::response Mo2Controller::get_plugin_action_status()
     * @brief Permits a transient mixed running and completed snapshot.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Result fields remain until the next action starts. Exit code -1 indicates
     * spawn failure and -2 indicates termination after the 30-minute timeout.
     *
     * @return HTTP 200 with the running state and last completed result.
     */
    crow::response get_plugin_action_status();

    /**
     * @fn crow::response Mo2Controller::run_plugin_action(const std::string&)
     * @brief Admits only fixed actions and screened path inputs.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The configured mods path falls back to `SALMA_MODS_PATH`. The background
     * worker terminates the child after 30 minutes.
     *
     * @param action `deploy` or `purge`.
     * @return The action start response.
     */
    crow::response run_plugin_action(const std::string& action);

    /**
     * @fn crow::response Mo2Controller::get_logs(const crow::request&)
     * @brief Advances byte offsets only across newline-terminated records.
     * @author Alex (<https://github.com/lextpf>)
     *
     * `offset` is a byte position. Bytes after the last newline are retained for
     * the next poll. An offset past EOF returns `reset` with offset zero.
     *
     * @param req Optional `lines` and `offset` query values. `lines` defaults to
     *            100, accepts zero for all records, and is capped at 500000.
     * @return HTTP 200 with lines, per-response counts, and the next byte offset.
     *         Returned records come from at most 12 MiB. The initial tail search
     *         can scan farther to locate the requested number of lines.
     */
    crow::response get_logs(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_test_logs(const crow::request&)
     * @brief Uses the application-log byte-offset protocol for test output.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param req Query values described by `get_logs`.
     * @return The same response shape and limits as `get_logs`.
     */
    crow::response get_test_logs(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::clear_logs()
     * @brief Serializes truncation with active file writes.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Rotated files remain. An incremental reader receives a reset only when its
     * saved offset exceeds the new file size.
     *
     * @return HTTP 200 on success or 500 on failure.
     */
    crow::response clear_logs();

    /**
     * @fn crow::response Mo2Controller::clear_test_logs()
     * @brief Can race an active test child that still owns the file.
     * @author Alex (<https://github.com/lextpf>)
     *
     * This is not synchronized with an active test child.
     *
     * @return HTTP 200 on success or 500 on failure.
     */
    crow::response clear_test_logs();

    /**
     * @fn crow::response Mo2Controller::run_tests(const crow::request&)
     * @brief Start one test child with screened command-line arguments.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Optional arguments permit only alphanumeric characters, spaces, `_`, `-`,
     * and single dots. Malformed JSON starts a run without arguments. The child
     * inherits the environment and uses the executable directory.
     *
     * @param req Optional JSON body with an `args` string.
     * @return HTTP 200 with the PID, 400 for unsafe arguments, 404 when the script
     *         is missing, 409 when busy, 500 on spawn failure, or 501 outside Windows.
     */
    crow::response run_tests(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_test_status()
     * @brief Reap a completed test child and report its exit code once.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The exit code appears only on the poll that first observes completion.
     *
     * @return HTTP 200 with running state and one-shot completion data.
     */
    crow::response get_test_status();

private:
    std::mutex test_mutex_;     // Guards test_process_
    bool test_running_{false};  // True while test_all.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test_all.py
#endif

    // A complete scan assigns each folder to one skip or processed counter.
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

    struct PluginActionResult
    {
        bool success = false;
        // -1 is spawn failure. -2 is termination after the 30-minute limit.
        int exit_code = 0;
        bool plugin_installed = false;
        std::string deploy_path;
        std::string action;
    };

    struct CachedResponse
    {
        nlohmann::json data;
        std::chrono::steady_clock::time_point timestamp{};

        /**
         * @fn bool CachedResponse::is_fresh(std::chrono::seconds ttl) const
         * @brief Check cache age with the monotonic clock.
         * @author Alex (<https://github.com/lextpf>)
         *
         * @param ttl Maximum age in seconds; zero expires every value.
         * @return `true` for a timestamp younger than the limit.
         * @pre The caller holds cache_mutex_.
         */
        [[nodiscard]] bool is_fresh(std::chrono::seconds ttl) const
        {
            return timestamp.time_since_epoch().count() > 0 &&
                   (std::chrono::steady_clock::now() - timestamp) < ttl;
        }

        /**
         * @fn void CachedResponse::set(nlohmann::json value)
         * @brief Replace the snapshot and start its cache lifetime.
         * @author Alex (<https://github.com/lextpf>)
         *
         * @param value Response data transferred into the cache.
         * @pre The caller holds cache_mutex_.
         */
        void set(nlohmann::json value)
        {
            data = std::move(value);
            timestamp = std::chrono::steady_clock::now();
        }

        /**
         * @fn void CachedResponse::invalidate()
         * @brief Expire the snapshot while retaining its data.
         * @author Alex (<https://github.com/lextpf>)
         *
         * @pre The caller holds cache_mutex_.
         */
        void invalidate() { timestamp = {}; }
    };

    mutable std::mutex cache_mutex_;
    CachedResponse fomods_cache_;
    CachedResponse status_cache_;

    BackgroundJob<ScanResult> scan_job_;
    BackgroundJob<PluginActionResult> plugin_action_job_;
};

}  // namespace mo2server

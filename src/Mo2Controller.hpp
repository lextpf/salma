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
 * @brief coordinates MO2 dashboard operations and background jobs.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Mo2Controller
 *
 * status and FOMOD listings are cached for 5 seconds. scan and plugin actions
 * use independent single-job slots. the test runner owns one child process.
 *
 * ### :material-state-machine: shutdown and test lifecycle
 *
 * shutdown requests cancellation before other state is destroyed. a cancelled
 * scan reports partial counters with success still set. the Win32 test child is
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
 * @warning worker lambdas must not capture this object or its members. a worker
 *          can detach after the 10-second shutdown grace period.
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
     * @fn crow::response Mo2Controller::get_config()
     * @brief includes path validity and the derived output directory.
     * @author Alex (https://github.com/lextpf)
     *
     * @return HTTP 200 with paths and a path-validity flag.
     */
    crow::response get_config();

    /**
     * @fn crow::response Mo2Controller::put_config(const crow::request&)
     * @brief rejects missing directories before changing live state.
     * @author Alex (https://github.com/lextpf)
     *
     * empty values, dot-dot components, and missing directories are rejected.
     * an absent key saves the current value again.
     *
     * @param req JSON body with optional `mo2ModsPath`.
     * @return HTTP 200, 400 for invalid input, or 500 when persistence fails.
     */
    crow::response put_config(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_status()
     * @brief serves five-second cached counts of top-level entries.
     * @author Alex (https://github.com/lextpf)
     *
     * JSON files and top-level mod directories are counted without recursion.
     *
     * @return HTTP 200 with a snapshot cached for 5 seconds.
     */
    crow::response get_status();

    /**
     * @fn crow::response Mo2Controller::list_fomods()
     * @brief bounds repeat filesystem work with a five-second cache.
     * @author Alex (https://github.com/lextpf)
     *
     * modified times use Unix epoch milliseconds. parse failures remain in the
     * list with `parseError` set and zero steps. order is filesystem-defined.
     *
     * @return HTTP 200 with a top-level listing cached for 5 seconds.
     */
    crow::response list_fomods();

    /**
     * @fn crow::response Mo2Controller::scan_fomods()
     * @brief rejects overlap and preserves existing selection files.
     * @author Alex (https://github.com/lextpf)
     *
     * the worker creates the output directory, skips existing selection files,
     * writes inferred files, and invalidates both response caches at completion.
     *
     * ### :material-transit-connection-variant: scan flow
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
     * @brief permits a transient mixed running and completed snapshot.
     * @author Alex (https://github.com/lextpf)
     *
     * a cancelled shutdown scan reports successful partial counters.
     *
     * @return HTTP 200 with running state and the last completed summary.
     */
    crow::response get_scan_status();

    /**
     * @fn crow::response Mo2Controller::get_fomod(const std::string&)
     * @brief enforces decoded-path containment before parsing.
     * @author Alex (https://github.com/lextpf)
     *
     * the name is URL-decoded and checked for containment. the response is
     * compact re-serialized JSON, not the original bytes.
     *
     * @param name URL-encoded mod name without the `.json` extension.
     * @return HTTP 200, 403 for traversal, 404 when missing, or 500 on read failure.
     */
    crow::response get_fomod(const std::string& name);

    /**
     * @fn crow::response Mo2Controller::delete_fomod(const std::string&)
     * @brief enforces decoded-path containment before permanent removal.
     * @author Alex (https://github.com/lextpf)
     *
     * deletion is permanent. the listing cache can retain the entry for 5 seconds.
     *
     * @param name URL-encoded mod name without the `.json` extension.
     * @return HTTP 200, 403 for traversal, 404 when missing, or 500 on failure.
     */
    crow::response delete_fomod(const std::string& name);

    /**
     * @fn crow::response Mo2Controller::deploy_plugin()
     * @brief runs the fixed deploy script through the single action slot.
     * @author Alex (https://github.com/lextpf)
     *
     * deploy and mods paths reject shell metacharacters before they enter the
     * batch environment. the script path is derived from the executable directory.
     *
     * @return HTTP 200 after start, 400 for unsafe paths, 404 when the script is
     *         missing, 409 when busy, or 501 outside Windows.
     */
    crow::response deploy_plugin();

    /**
     * @fn crow::response Mo2Controller::purge_plugin()
     * @brief runs the fixed purge script through the single action slot.
     * @author Alex (https://github.com/lextpf)
     *
     * @return HTTP 200 after start, with the same failures as `deploy_plugin`.
     * @warning purge permanently removes plugin files and the complete
     *          `Salma FOMODs Output` directory without confirmation.
     */
    crow::response purge_plugin();

    /**
     * @fn crow::response Mo2Controller::get_plugin_action_status()
     * @brief permits a transient mixed running and completed snapshot.
     * @author Alex (https://github.com/lextpf)
     *
     * result fields remain until the next action starts. exit code -1 indicates
     * spawn failure and -2 indicates termination after the 30-minute timeout.
     *
     * @return HTTP 200 with the running state and last completed result.
     */
    crow::response get_plugin_action_status();

    /**
     * @fn crow::response Mo2Controller::run_plugin_action(const std::string&)
     * @brief admits only fixed actions and screened path inputs.
     * @author Alex (https://github.com/lextpf)
     *
     * the configured mods path falls back to `SALMA_MODS_PATH`. the background
     * worker terminates the child after 30 minutes.
     *
     * @param action `deploy` or `purge`.
     * @return the action start response.
     */
    crow::response run_plugin_action(const std::string& action);

    /**
     * @fn crow::response Mo2Controller::get_logs(const crow::request&)
     * @brief advances byte offsets only across newline-terminated records.
     * @author Alex (https://github.com/lextpf)
     *
     * `offset` is a byte position. bytes after the last newline are retained for
     * the next poll. an offset past EOF returns `reset` with offset zero.
     *
     * @param req optional `lines` and `offset` query values. `lines` defaults to
     *            100, accepts zero for all records, and is capped at 500000.
     * @return HTTP 200 with lines, per-response counts, and the next byte offset.
     *         one response reads at most 12 MiB.
     */
    crow::response get_logs(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_test_logs(const crow::request&)
     * @brief uses the application-log byte-offset protocol for test output.
     * @author Alex (https://github.com/lextpf)
     *
     * @param req query values described by `get_logs`.
     * @return the same response shape and limits as `get_logs`.
     */
    crow::response get_test_logs(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::clear_logs()
     * @brief serializes truncation with active file writes.
     * @author Alex (https://github.com/lextpf)
     *
     * rotated files remain. incremental readers receive a reset on the next poll.
     *
     * @return HTTP 200 on success or 500 on failure.
     */
    crow::response clear_logs();

    /**
     * @fn crow::response Mo2Controller::clear_test_logs()
     * @brief can race an active test child that still owns the file.
     * @author Alex (https://github.com/lextpf)
     *
     * this is not synchronized with an active test child.
     *
     * @return HTTP 200 on success or 500 on failure.
     */
    crow::response clear_test_logs();

    /**
     * @fn crow::response Mo2Controller::run_tests(const crow::request&)
     * @brief rejects overlap and passes the optional filter through the environment.
     * @author Alex (https://github.com/lextpf)
     *
     * optional arguments permit only alphanumeric characters, spaces, `_`, `-`,
     * and single dots. malformed JSON starts a run without arguments. the child
     * inherits the environment and uses the executable directory.
     *
     * @param req optional JSON body with an `args` string.
     * @return HTTP 200 with the PID, 400 for unsafe arguments, 404 when the script
     *         is missing, 409 when busy, 500 on spawn failure, or 501 outside Windows.
     */
    crow::response run_tests(const crow::request& req);

    /**
     * @fn crow::response Mo2Controller::get_test_status()
     * @brief preserves the final result after reaping the child.
     * @author Alex (https://github.com/lextpf)
     *
     * the exit code appears only on the poll that first observes completion.
     *
     * @return HTTP 200 with running state and one-shot completion data.
     */
    crow::response get_test_status();

private:
    std::mutex test_mutex_;     // guards test_process_
    bool test_running_{false};  // true while test_all.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test_all.py
#endif

    // a complete scan assigns each folder to one skip or processed counter.
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

    BackgroundJob<ScanResult> scan_job_;
    BackgroundJob<PluginActionResult> plugin_action_job_;
};

}  // namespace mo2server

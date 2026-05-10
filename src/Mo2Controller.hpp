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
 * @brief REST endpoints for the MO2 integration dashboard.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Mo2Controller
 *
 * Serves dashboard configuration, FOMOD JSON browsing, MO2 integration
 * status, log tailing, plugin deploy and purge, and test execution.
 * Routes are registered in `main.cpp`; the table below is the contract
 * the dashboard codes against.
 *
 * That table and the response shapes are maintained by hand. Change a
 * route in `main.cpp` and change this block in the same commit, or the
 * two drift apart with nothing to catch it.
 *
 * ## :material-api: Endpoints
 *
 * | Method | Route                         | Handler                  |
 * |--------|-------------------------------|--------------------------|
 * |    GET | `/api/config`                 | get_config               |
 * |    PUT | `/api/config`                 | put_config               |
 * |    GET | `/api/mo2/status`             | get_status               |
 * |    GET | `/api/mo2/fomods`             | list_fomods              |
 * |   POST | `/api/mo2/fomods/scan`        | scan_fomods              |
 * |    GET | `/api/mo2/fomods/scan/status` | get_scan_status          |
 * |    GET | `/api/mo2/fomods/<name>`      | get_fomod                |
 * | DELETE | `/api/mo2/fomods/<name>`      | delete_fomod             |
 * |   POST | `/api/plugin/deploy`          | deploy_plugin            |
 * |   POST | `/api/plugin/purge`           | purge_plugin             |
 * |    GET | `/api/plugin/status`          | get_plugin_action_status |
 * |    GET | `/api/logs`                   | get_logs                 |
 * |    GET | `/api/logs/test`              | get_test_logs            |
 * |   POST | `/api/logs/clear`             | clear_logs               |
 * |   POST | `/api/logs/clear/test`        | clear_test_logs          |
 * |   POST | `/api/test/run`               | run_tests                |
 * |    GET | `/api/test/status`            | get_test_status          |
 *
 * ## :material-code-braces: Request and response shapes
 *
 * Type key: `"..."` is a string, `bool` a boolean, `int` an integer, and
 * a trailing `?` marks an optional field. Every error response is
 * `{ "error": "..." }` with a matching HTTP status code.
 *
 * ### Configuration
 *
 * `GET /api/config` returns
 * `{ "mo2ModsPath", "fomodOutputDir", "mo2ModsPathValid": bool }`.
 *
 * `PUT /api/config` takes `{ "mo2ModsPath": "..." }` and returns the same
 * shape.
 *
 * ### MO2 integration
 *
 * `GET /api/mo2/status` returns `{ "configured": bool,
 * "outputFolderExists": bool, "fomodOutputDir", "jsonCount": int,
 * "modCount": int, "pluginInstalled": bool, "pluginDeployPath" }`.
 *
 * `GET /api/mo2/fomods` returns `[{ "name", "size": int, "modified": int,
 * "stepCount": int, "confidence"?: float, "confidenceBand"?: "...",
 * "exactMatch"?: bool, "parseError"?: true }]`.
 *
 * - `size` is bytes. `modified` is milliseconds since the Unix epoch, not
 *   seconds; a client that assumes seconds renders every date in 1970.
 * - The three confidence fields appear only for files whose diagnostics
 *   block carries them.
 * - `parseError` appears only when the step-counting pass over that file
 *   threw, and `stepCount` is then 0.
 * - An unconfigured or missing output directory yields `[]` with 200.
 *   Cached for 5 seconds, so a just-written file can stay invisible that
 *   long.
 *
 * `POST /api/mo2/fomods/scan` returns
 * `{ "success": true, "running": true, "started": true }`. Errors: 400
 * bad path, 409 busy, 500 when the output directory cannot be created.
 *
 * `GET /api/mo2/fomods/scan/status` returns `{ "running": bool,
 * "success": bool, "totalModFolders": int, "archivesProcessed": int,
 * "choicesInferred": int, "noFomod": int, "alreadyHadChoices": int,
 * "noArchiveFound": int, "archiveMissing": int, "errors": int,
 * "durationMs": int, "outputDir", "error"? }`. Everything after `running`
 * appears only once a scan has completed, and then keeps describing that
 * last finished scan until a new scan starts.
 *
 * `GET /api/mo2/fomods/<name>` returns the FOMOD JSON parsed and
 * re-serialised compactly, not the file bytes: the on-disk two-space
 * indentation is gone and object keys come out in nlohmann's sorted
 * order rather than the file's. `<name>` is URL-decoded, then `.json` is
 * appended. Errors: 403 traversal, 404 output directory unconfigured or
 * file missing, 500 read or parse failure.
 *
 * `DELETE /api/mo2/fomods/<name>` returns `{ "success": true }`. Errors:
 * 403 traversal, 404 output directory unconfigured or file missing, 500
 * delete failure.
 *
 * ### Plugin management
 *
 * `POST /api/plugin/deploy` and `POST /api/plugin/purge` return
 * `{ "started": true, "action": "deploy"|"purge" }`. Errors: 400 mods
 * path unconfigured or a path holds a shell metacharacter, 404 script
 * missing, 409 busy, 501 non-Windows.
 *
 * `GET /api/plugin/status` returns `{ "running": bool, "success": bool,
 * "exitCode": int, "pluginInstalled": bool, "pluginDeployPath", "action",
 * "error"? }`. Everything after `running` appears only once an action has
 * completed, and then keeps describing that last finished action until a
 * new one starts. Read `action` to know whether it was a deploy or a
 * purge.
 *
 * ### Logs
 *
 * `GET /api/logs` and `GET /api/logs/test` take `?lines=N` (default 100,
 * max 500000) and `?offset=B` for the incremental tail. `lines=0` means
 * no trimming: return every line in the window. A negative value clamps
 * to 0. A value that does not parse as an integer falls back to 100 with
 * no error.
 *
 * Both return `{ "lines": ["..."], "errors": int, "warnings": int,
 * "passes": int, "nextOffset": int, "reset"?: true }`.
 *
 * - A polling client has to handle `reset: true`. It is sent when the
 *   requested `offset` is past the end of the file, which means the log
 *   was cleared or rotated. `nextOffset` is then 0 and the client has to
 *   restart from offset 0.
 * - One response carries at most 12 MiB of log bytes. A longer tail takes
 *   several requests through the incremental-offset protocol.
 * - `errors`, `warnings` and `passes` count only the lines in this one
 *   response, not the whole file, and the three are mutually exclusive: a
 *   line is classified by the first keyword group that matches, so a line
 *   holding both `ERROR` and `WARNING` counts once, as an error.
 *
 * `POST /api/logs/clear` and `POST /api/logs/clear/test` return
 * `{ "success": true }`.
 *
 * ### Test runner
 *
 * `POST /api/test/run` takes `{ "args"?: "..." }` and returns
 * `{ "running": true, "pid": int }`. Errors: 400 disallowed characters or
 * `..` in `args`, 404 `test_all.py` missing, 409 busy, 500 CreateProcess
 * failure, 501 non-Windows.
 *
 * `GET /api/test/status` returns
 * `{ "running": bool, "exitCode"?: int, "error"?: "..." }`.
 *
 * - `exitCode` appears on the poll that first observes the process has
 *   finished. That same poll closes the handle, so the following poll
 *   reports only `{ "running": false }` and the exit code is gone.
 * - `error` appears instead of `exitCode` when the process query itself
 *   failed. The handle is closed and the run forgotten in that case too.
 *
 * ## :material-sync: Scan job lifecycle
 *
 * `POST /api/mo2/fomods/scan` submits work to `scan_job_`
 * (`BackgroundJob<ScanResult>`), whose worker thread walks every mod
 * folder under the configured MO2 mods path and infers FOMOD selections.
 *
 * 1. Start: `scan_job_.try_start(work)` returns 200 immediately, or 409
 *    when `scan_job_.is_running()` reports a scan already in progress.
 * 2. Poll: `GET /api/mo2/fomods/scan/status` checks
 *    `scan_job_.is_running()` and, once the worker has finished, reads
 *    `scan_job_.read_result()` for the full summary.
 * 3. Completion: `BackgroundJob` stores the `ScanResult` under its own
 *    mutex and clears the running flag.
 * 4. Cancellation: the destructor calls `scan_job_.shutdown()`, which
 *    sets the cancel token. The worker checks it once per mod folder and
 *    breaks out of the loop. It still stores a summary with
 *    `"success": true` and partial counts, so a client cannot tell a
 *    cancelled scan from a completed one. The log line
 *    `[infer] Scan cancelled by shutdown request` is the only marker.
 *
 * The worker is joined at shutdown, or detached if it outlives
 * `BackgroundJob::kShutdownGrace` (10 s).
 *
 * Two selection rules run before a folder is inferred, and they count
 * differently. `Salma FOMODs Output` is dropped from the folder list
 * outright, so it never reaches `totalModFolders`. A mod that already has
 * a choices JSON in the output directory does count in `totalModFolders`
 * but is skipped rather than re-inferred: it lands in
 * `alreadyHadChoices`, not `archivesProcessed`. Delete that JSON to force
 * a re-scan of one mod.
 *
 * Plugin deploy and purge follow the same `BackgroundJob` pattern through
 * `plugin_action_job_` (`BackgroundJob<PluginActionResult>`).
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
 *       Job-->>Ctrl: running plus ScanResult when finished
 *       Ctrl-->>C: 200 status
 *     end
 *     opt server shutdown
 *       Ctrl->>Job: shutdown sets the cancel token
 *       Job-->>Ctrl: partial summary, still success true
 *     end
 * ```
 *
 * ## :material-microsoft-windows: Test runner process lifecycle
 *
 * `POST /api/test/run` does not use `BackgroundJob`. It launches a Win32
 * process with `CreateProcessA` and tracks it through a raw `HANDLE`
 * (`test_process_`), which `get_test_status()` polls using
 * `WaitForSingleObject`. The `test_running_` flag and `test_mutex_` are
 * managed by hand.
 *
 * `WaitForSingleObject` is used rather than `GetExitCodeProcess` plus a
 * `STILL_ACTIVE` test, because a process that genuinely exits with code
 * 259 is indistinguishable from a running one under that test.
 *
 * The ANSI entry point builds the command line as a narrow
 * `std::string`. A character in the executable directory path that the
 * active ANSI code page cannot represent therefore cannot reach the
 * command line, and the spawn fails with 500.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * stateDiagram-v2
 *     [*] --> idle
 *     idle --> running: POST /api/test/run, CreateProcessA ok, 200 running plus pid
 *     idle --> idle: 400 bad args, 404 test_all.py missing, 500 CreateProcess failed
 *     running --> running: GET /api/test/status, WAIT_TIMEOUT, running true
 *     running --> idle: process exited, GetExitCodeProcess, running false plus exitCode
 *     running --> idle: WAIT_FAILED, handle closed, running false plus error
 *     running --> idle: POST /api/test/run reaps an already finished process, new run starts
 *     running --> [*]: destructor terminates the child and waits up to 5 s
 * ```
 *
 * ## :material-source-branch: Implementation file split
 *
 * Handler bodies live in separate translation units, grouped by concern
 * to keep each file near 300 lines or under.
 *
 * | Translation unit            | Endpoints                                      |
 * |-----------------------------|------------------------------------------------|
 * | `Mo2Controller.cpp`         | constructor, destructor, `/api/mo2/status`     |
 * | `Mo2ConfigController.cpp`   | `/api/config` (GET, PUT)                       |
 * | `Mo2FomodController.cpp`    | `/api/mo2/fomods...` (list, scan, get, delete) |
 * | `Mo2LogController.cpp`      | `/api/logs/...`                                |
 * | `Mo2PluginController.cpp`   | `/api/plugin/...`                              |
 * | `Mo2TestController.cpp`     | `/api/test/...` plus Win32 process glue        |
 * | `Mo2Helpers.hpp/.cpp`       | shared helpers (json_response, paths)          |
 *
 * Add a new handler to the file that owns its endpoint group rather than
 * to `Mo2Controller.cpp`. One handler already breaks that rule:
 * `get_status` (`GET /api/mo2/status`) sits in `Mo2Controller.cpp`, not
 * in `Mo2FomodController.cpp`. Grep for the method name, not the route
 * prefix, when hunting for a handler body.
 *
 * ## :material-warehouse: Shutdown invariant
 *
 * Member declaration order carries no meaning here. The destructor pins
 * teardown order in code: it calls `shutdown()` on `scan_job_` and then
 * on `plugin_action_job_` before any other member is destroyed, and only
 * then terminates a still-running test child.
 *
 * One rule follows for anyone adding a background job: a worker lambda
 * must not capture `this` or any member by reference. A worker that does
 * not observe the cancel token inside `BackgroundJob::kShutdownGrace`
 * (10 s) is detached rather than joined, so it can still be running after
 * the controller is gone. Detaching is safe only because `BackgroundJob`
 * hands the worker a `shared_ptr` to its own state, which outlives
 * `*this`.
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
     *
     * Never cached and never fails: an unconfigured or deleted mods path
     * is reported as `"mo2ModsPathValid": false`, not as an error.
     *
     * @return 200 with config object.
     */
    crow::response get_config();

    /**
     * @brief Validates and persists a new MO2 mods path, then returns the updated config.
     *
     * The traversal check rejects `..` only as a complete path segment,
     * so a directory whose name merely contains two dots (`My..Mod`) is
     * accepted. The write is transactional: `ConfigService` stages the
     * value in memory, saves `salma.json`, and reverts the in-memory
     * value if the save fails, so process state cannot diverge from disk.
     *
     * A body with no `mo2ModsPath` key is not an error. It is treated as
     * a re-save of the current configuration.
     *
     * @param req JSON body with `"mo2ModsPath"`. Rejects empty values, `..` segments, and
     * non-existent directories.
     * @return 200 with updated config, or 400/500 on validation/persistence failure.
     *         A body that is not valid JSON also returns 400, with the
     *         generic message `Invalid request`.
     */
    crow::response put_config(const crow::request& req);

    /**
     * @brief Reports MO2 integration health: whether paths are configured, JSON/mod counts, and
     * plugin install state.
     *
     * `jsonCount` counts `.json` files directly in the FOMOD output
     * directory. `modCount` counts top-level directories under the mods
     * path, so it includes `Salma FOMODs Output` and any other non-mod
     * folder. Both are 0 when the corresponding directory is
     * unconfigured or missing; neither condition is an error.
     *
     * @return 200 with status object. Cached for 5 seconds. The cache is
     * invalidated when a scan finishes.
     */
    crow::response get_status();

    /**
     * @brief Lists all FOMOD JSON files in the output directory with metadata (size, modified
     * time, step count).
     *
     * Only the top level of the output directory is read; there is no
     * recursion. Each file is walked with a SAX pass that counts the
     * top-level `steps` entries and picks up the diagnostics fields, so
     * the cost scales with the step count, not the file size. A file that
     * fails to parse is still listed, with `parseError: true`.
     *
     * @return 200 with JSON array of FOMOD summaries. Returns empty array if the output directory
     * is missing. Cached for 5 seconds. Order follows the directory
     * iteration order, which is not sorted.
     */
    crow::response list_fomods();

    /**
     * @brief Kicks off an async background scan that infers FOMOD selections for every mod under
     * the MO2 mods path.
     *
     * Returns as soon as the worker starts and does not wait for the
     * scan. Creates the FOMOD output directory if it does not exist. The
     * scan writes one `<mod name>.json` per mod it infers and invalidates
     * both response caches when it finishes.
     *
     * @return 200 if the scan started, 400 if paths are misconfigured, 409 if a scan is already
     * running, 500 on mkdir failure.
     */
    crow::response scan_fomods();

    /**
     * @brief Polls the running/completed state of the last FOMOD scan job.
     *
     * The summary describes the last scan that stored a result, and a
     * scan cancelled at shutdown stores one too: it reports
     * `"success": true` with partial counts. See the scan job lifecycle
     * section above.
     *
     * @return 200 with `running` flag; includes full scan summary (counts, duration) once
     * finished.
     */
    crow::response get_scan_status();

    /**
     * @brief Returns the parsed contents of a single FOMOD JSON file by mod name.
     *
     * The file is parsed and re-serialised compactly, so the response is
     * not the file bytes: the on-disk two-space indentation is lost. A
     * caller that needs the exact file has to read it from disk.
     *
     * @param name URL-encoded mod name (without `.json` extension). It is
     *        URL-decoded, then joined to the FOMOD output directory and
     *        re-checked with `mo2core::is_inside`.
     * @return 200 with the re-serialised JSON, 403 on path traversal,
     *         404 when the output directory is unconfigured or the file
     *         does not exist, 500 when the file cannot be read or parsed.
     */
    crow::response get_fomod(const std::string& name);

    /**
     * @brief Deletes a single FOMOD JSON file from the output directory.
     *
     * Deletion is permanent; nothing is moved to a recycle bin. The
     * `fomods_cache_` entry is left alone here, so `GET /api/mo2/fomods`
     * can keep listing the deleted file for up to 5 seconds.
     *
     * @param name URL-encoded mod name (without `.json` extension). Same
     *        decoding and containment check as get_fomod().
     * @return 200 on success, 403 on path traversal, 404 when the output
     *         directory is unconfigured or the file does not exist, 500
     *         when the delete fails.
     */
    crow::response delete_fomod(const std::string& name);

    /**
     * @brief Runs the `deploy.bat` script in the background to install the Salma MO2 plugin.
     *
     * **Path screening.** Both the deploy path and the configured MO2
     * mods path are scanned for shell metacharacters before launch. The
     * blocked set is a whitelist complement: `& | > < ^ % ! ( ) " ; ' \``.
     * Any of those characters in either path yields 400.
     *
     * Neither path reaches the child's command line. Both are passed as
     * `SALMA_DEPLOY_PATH` and `SALMA_MODS_PATH` entries in an explicit
     * environment block. The screen exists to stop `%VAR%` expansion
     * inside `deploy.bat` and `purge.bat` from producing text that
     * `cmd.exe` then re-parses as syntax. It is not protection against
     * argv splitting, because there is no argv to split.
     *
     * The script path is the single value that does reach the command
     * line, and it is never metachar-checked. It is trusted because it is
     * built from `mo2core::executable_directory()` plus a fixed filename,
     * never from request input. If that stops being true, extend the
     * screen to cover it.
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
     * **This deletes user data.** Besides removing `salma/mo2-salma.dll`,
     * `mo2-salma.py` and the plugin-side log, `purge.bat` recursively
     * deletes the whole `<mods path>/Salma FOMODs Output` folder, which
     * holds every inferred selections JSON. There is no confirmation
     * step and nothing is moved to a recycle bin.
     *
     * @return 200 if started, 400 if mods path unconfigured or contains shell metacharacters, 404
     * if script missing, 409 if busy, 501 on non-Windows.
     */
    crow::response purge_plugin();

    /**
     * @brief Polls the running/completed state of the last plugin deploy or purge action.
     *
     * The result slot is not cleared between actions, so after a deploy
     * finishes this keeps reporting that deploy until a purge starts.
     * Read `action` to know which one the numbers describe.
     *
     * @return 200 with `running` flag; includes exit code, install state, and action name once
     * finished. `exitCode` is the batch script's own exit code, except
     * for two sentinels: -1 means `CreateProcessA` failed and -2 means
     * the script was killed on the 30-minute timeout.
     */
    crow::response get_plugin_action_status();

    /**
     * @brief Shared implementation for deploy_plugin() and purge_plugin(). Validates paths, then
     * launches the corresponding batch script via BackgroundJob.
     *
     * The script is resolved as `<executable directory>/<action>.bat`,
     * so the server directory must hold `deploy.bat` and `purge.bat`.
     * The mods path comes from `ConfigService`, falling back to the
     * `SALMA_MODS_PATH` environment variable when the config is empty.
     *
     * **Blocking**: the batch script itself runs on the background
     * thread and is waited on for at most 30 minutes; on timeout the
     * child is terminated and the result carries `exitCode` -2.
     *
     * @param action Must be `"deploy"` or `"purge"`; any other value returns 400.
     * @return 200 if started, 400/404/409/501 on error (see deploy_plugin/purge_plugin docs).
     */
    crow::response run_plugin_action(const std::string& action);

    /**
     * @brief Returns the tail of `logs/salma.log`. Supports incremental reads via `offset` query
     * param.
     *
     * The file is `Logger::log_path()`, which anchors `logs/salma.log`
     * beside the owning module rather than resolving it through the
     * working directory.
     *
     * Only whole lines are returned. Bytes after the last newline in the
     * read window are dropped and are not counted into `nextOffset`, so a
     * half-written entry is never handed to the client and arrives
     * complete on the next poll.
     *
     * @param req Query params: `?lines=N` (default 100, max 500000, `0`
     * means no trimming, unparseable falls back to 100), `?offset=B` in
     * bytes for incremental mode. A missing or negative `offset` selects
     * full mode, which returns the final `lines` entries.
     * @return 200 with `lines` array, keyword counts (errors/warnings/passes), and `nextOffset`
     * for polling. Adds `"reset": true` when `offset` is past the end of
     * the file. A missing log file is reported as an empty successful
     * response, not 404. At most 12 MiB of log bytes per response.
     */
    crow::response get_logs(const crow::request& req);

    /**
     * @brief Returns the tail of `test.log`. Supports incremental reads via `offset` query param.
     *
     * Same reader, same contract and same limits as get_logs(); only the
     * file differs. It is `<executable directory>/test.log`, written by
     * the `test_all.py` child that run_tests() spawns.
     *
     * @param req Query params: `?lines=N` (default 100, max 500000, `0`
     * means no trimming, unparseable falls back to 100), `?offset=B` in
     * bytes for incremental mode.
     * @return 200 with `lines` array, keyword counts (errors/warnings/passes), and `nextOffset`
     * for polling. Adds `"reset": true` when `offset` is past the end of
     * the file. At most 12 MiB of log bytes per response.
     */
    crow::response get_test_logs(const crow::request& req);

    /**
     * @brief Truncates `logs/salma.log` via Logger::clear_log() to coordinate with the persistent
     * file handle.
     *
     * Truncation goes through the Logger because the Logger holds the
     * file open. Truncating behind its back would leave the next write at
     * a stale offset. Rotated files (`.1` to `.3`) are left in place. The
     * clear itself is logged, so the file is not empty for long.
     *
     * A client polling with `?offset=` sees `"reset": true` on its next
     * request and has to restart from offset 0.
     *
     * @return 200 on success, 500 if truncation fails.
     */
    crow::response clear_logs();

    /**
     * @brief Truncates `test.log` by opening it in trunc mode.
     *
     * `test.log` is written by the `test_all.py` child, not by the
     * Logger, so there is no in-process file handle to coordinate with
     * and plain truncation is used. The file is resolved as
     * `<executable directory>/test.log`, which is where `test_all.py`
     * writes it. Nothing checks whether a test run is in flight, so the
     * result of clearing during a run depends on how the child holds the
     * file; do not rely on it.
     *
     * @return 200 on success, 500 if truncation fails.
     */
    crow::response clear_test_logs();

    /**
     * @brief Spawns `test_all.py` as a child Win32 process whose handle the controller keeps. Only
     * one test run may be active at a time.
     *
     * Unlike scan_fomods(), deploy_plugin() and purge_plugin(), this
     * endpoint uses no `BackgroundJob`; see the test runner process
     * lifecycle section above for the raw-handle scheme it uses instead.
     *
     * The run does not survive server shutdown: the destructor
     * terminates a child that is still running, waits up to 5 seconds
     * for it, and closes the handle.
     *
     * A previous run that has already exited is reaped here, so two calls
     * in a row succeed as long as the first child is gone.
     *
     * The child inherits the server's environment and runs with the
     * executable directory as its working directory. It writes its own
     * `test.log`, which get_test_logs() then serves.
     *
     * The optional `"args"` string is matched against the whitelist regex
     * `^[a-zA-Z0-9 _\-\.]*$`: alphanumeric, space, underscore, hyphen and
     * dot. Quotes and path separators are excluded so an argument cannot
     * escape its boundary or name a path. `.` alone is allowed, so the
     * `..` substring is rejected by a separate check. An empty string
     * passes, which is what makes omitting `args` legal.
     *
     * A body that is not valid JSON, or one whose `args` is not a string,
     * is not an error: a warning is logged and the run starts with no
     * arguments.
     *
     * @param req Optional JSON body with `"args"` (whitelist-sanitized as above).
     * @return 200 with PID on success, 400 on bad args, 404 if test_all.py missing, 409 if already
     * running, 500 on CreateProcess failure, 501 on non-Windows.
     */
    crow::response run_tests(const crow::request& req);

    /**
     * @brief Checks whether the test_all.py process is still running. Cleans up the process handle
     * on completion.
     *
     * The cleanup destroys the state it reports: the poll that first sees
     * the child has exited closes the handle and clears `test_running_`,
     * so `exitCode` is reported exactly once. A client that misses it
     * cannot ask again.
     *
     * @return 200 with `running` flag; includes `exitCode` on the single
     * poll that observes completion, or `error` instead when the process
     * query itself failed. On a non-Windows build this always reports
     * `{ "running": false }`.
     */
    crow::response get_test_status();

private:
    // -- Test runner state --
    std::mutex test_mutex_;     // guards test_process_
    bool test_running_{false};  // true while test_all.py is executing
#ifdef _WIN32
    HANDLE test_process_{nullptr};  // Win32 process handle for test_all.py
#endif

    /// @brief Summary of one FOMOD scan job, mapped 1:1 onto the
    /// `/api/mo2/fomods/scan/status` body.
    ///
    /// Every mod folder lands in one and only one of the skip and
    /// outcome counters, so a scan that ran to the end satisfies
    /// `already_had_choices + no_archive_found + archive_missing +
    /// archives_processed == total_mod_folders`. A cancelled scan reports
    /// partial counts with `success` still true.
    struct ScanResult
    {
        bool success = false;         ///< False only when the mods directory could not be listed.
        int total_mod_folders = 0;    ///< Folders found, excluding `Salma FOMODs Output`.
        int archives_processed = 0;   ///< Folders that reached inference: inferred + no_fomod +
                                      ///< errors.
        int choices_inferred = 0;     ///< Folders for which a choices JSON was written.
        int no_fomod = 0;             ///< Archives that hold no FOMOD installer.
        int already_had_choices = 0;  ///< Skipped: a choices JSON already existed. Not re-inferred.
        int no_archive_found = 0;     ///< Skipped: `meta.ini` names no installation file.
        int archive_missing = 0;      ///< Skipped: the named archive did not resolve on disk.
        int errors = 0;               ///< Folders whose inference threw.
        long long duration_ms = 0;    ///< Wall-clock scan time in milliseconds.
        std::string output_dir;       ///< Directory the choices JSONs were written to.
    };

    /// @brief Outcome of one plugin deploy or purge action, mapped 1:1
    /// onto the `/api/plugin/status` body.
    struct PluginActionResult
    {
        bool success = false;           ///< True only when `exit_code` is 0.
        int exit_code = 0;              ///< Script exit code, or -1 spawn failed, -2 killed on the
                                        ///< 30-minute timeout.
        bool plugin_installed = false;  ///< Re-probed once the script has run, so it is the state
                                        ///< the action left behind.
        std::string deploy_path;        ///< Path the script was pointed at.
        std::string action;             ///< `"deploy"` or `"purge"`. Says which run these fields
                                        ///< describe.
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
    // Declaration order relative to the cache members carries no meaning.
    // The destructor calls shutdown() on both jobs first, so teardown order
    // is pinned in code rather than left to reverse-order destruction.
    // A worker that misses BackgroundJob::kShutdownGrace is detached, not
    // joined, so a worker lambda must never capture `this` or a member.
    BackgroundJob<ScanResult> scan_job_;
    BackgroundJob<PluginActionResult> plugin_action_job_;
};

}  // namespace mo2server

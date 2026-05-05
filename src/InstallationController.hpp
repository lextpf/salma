#pragma once

#include <crow.h>
#include <nlohmann/json.hpp>

#include <optional>
#include <string>

#include "BackgroundJob.hpp"

namespace mo2server
{

/**
 * @struct InstallJobResult
 * @brief Result payload stored by the background installation job.
 * @author Alex (https://github.com/lextpf)
 * @ingroup InstallationController
 *
 * Distinct from the engine's own install result because the job also
 * carries `mod_name`, derived from the upload filename or from user
 * input, which the engine never returns.
 *
 * The job thread writes the struct once; readers take it under
 * `BackgroundJob`'s mutex afterwards. Exactly one of two outcomes
 * applies: `success == true` with `mod_path` set, or `success == false`
 * with `error` set from the engine's exception message.
 */
struct InstallJobResult
{
    bool success = false;  ///< True only when the engine reported a completed install.
    std::string mod_path;  ///< Install path the engine reported, not the one requested.
    std::string mod_name;  ///< Resolved mod name. Empty for the `/install` route.
    std::string error;     ///< Engine exception message. Empty when `success` is true.
};

/**
 * @class InstallationController
 * @brief REST endpoints for upload, install, and status.
 * @author Alex (https://github.com/lextpf)
 * @ingroup InstallationController
 *
 * Handles HTTP requests from the React frontend for the server-based
 * installation workflow. Parses multipart uploads with MultipartHandler,
 * delegates the install to SalmaEngine, and returns JSON responses.
 *
 * SalmaEngine loads `mo2-salma.dll` and calls it over the flat
 * `extern "C"` ABI, where the work lands in the engine's
 * `InstallationService::install_mod`. SalmaEngine restores the throwing
 * failure mode that the flat ABI erases, so a failed install raises
 * `std::runtime_error` carrying the engine's own message. The background
 * job catches it and stores it as InstallJobResult::error.
 *
 * ## :material-api: Endpoints
 *
 * | Method |              Route              | Handler                                          |
 * |--------|---------------------------------|--------------------------------------------------|
 * |   POST |   `/api/installation/upload`    | handle_upload - receive archive + install        |
 * |   POST |   `/api/installation/install`   | handle_install - install from existing path      |
 * |    GET | `/api/installation/status/<id>` | handle_status - poll the single install job slot |
 *
 * All three are registered in `main.cpp`. The controller owns a single
 * job slot, so `upload` and `install` compete for it: whichever arrives
 * while the other is running gets 409.
 *
 * ## :material-code-braces: Response shapes
 *
 * | Route              | Status  | Body                                                        |
 * |--------------------|---------|-------------------------------------------------------------|
 * | `POST .../upload`  | **200** | `{ "started": true, "modName": "..." }`                     |
 * |                    | **400** | `{ "error": "..." }` - see the validation list below        |
 * |                    | **409** | `{ "error": "An installation is already running" }`         |
 * |                    | **413** | `{ "error": "Upload exceeds 8 GiB limit" }`                 |
 * | `POST .../install` | **200** | `{ "started": true }`                                       |
 * |                    | **400** | `{ "error": "..." }` - see the validation list below        |
 * |                    | **409** | `{ "error": "An installation is already running" }`         |
 * | `GET .../status/x` | **200** | `{ "running": bool, "success"?: bool, "modPath"?: "...",    |
 * |                    |         |   "modName"?: "...", "error"?: "..." }`                     |
 * | *(any)*            | **500** | `{ "error": "..." }`                                        |
 *
 * `modName` appears in the status body only for an install started by
 * `/upload`; the `/install` route never sets it.
 *
 * ## :material-shield-outline: Validation rejections (400)
 *
 * Each rejection below returns 400 with its own distinct message, so a
 * dashboard that shows the raw `error` string needs no further mapping.
 *
 * `POST /api/installation/upload`:
 *
 * - `No file uploaded or file is empty` - no `file` part, or the write failed.
 * - `modName contains path separators, traversal, or reserved characters`
 * - `Cannot validate modPath: MO2 mods directory is not configured`
 * - `modPath must be inside the configured MO2 mods directory`
 * - `No modPath provided and MO2 mods path is not configured`
 * - `Generated mod path escapes the configured MO2 mods directory`
 *   (defense in depth; unreachable while the modName check holds)
 *
 * `POST /api/installation/install`:
 *
 * - `ArchivePath and ModPath are required`
 * - `Archive path does not exist`
 * - `archivePath must be inside the configured mods or downloads directory`
 * - `Cannot validate modPath: MO2 mods directory is not configured`
 * - `modPath must be inside the configured MO2 mods directory`
 * - `jsonPath does not exist`
 * - `jsonPath must be inside the configured mods, downloads, FOMOD output, or archive directory`
 *
 * **Containment roots** for `/install` differ per field:
 *
 * | Field | Must sit under |
 * |-------|----------------|
 * | `archivePath` | the configured MO2 mods path, or `SALMA_DOWNLOADS_PATH` |
 * | `modPath` | the configured MO2 mods path |
 * | `jsonPath` | any of: mods path, downloads path, FOMOD output dir, archive's own dir |
 *
 * The same policy is restated as a table at the top of
 * InstallationController.cpp. This block is the reference for a caller;
 * the two have to stay in step.
 *
 * `SALMA_DOWNLOADS_PATH` is an external input the client cannot see, yet
 * it decides which requests succeed. It is ignored unless it is an
 * absolute non-root path: a root value would make the containment test a
 * no-op, and a relative one would resolve against whatever working
 * directory the process happens to have. The `archivePath` check logs a
 * warning when it ignores the variable; the `jsonPath` check applies the
 * same rule with no log line, so a mis-set variable narrows the
 * `jsonPath` roots with nothing to explain it.
 *
 * When no root at all is configured, the containment test is skipped.
 * The `archivePath` check logs a warning when it skips; the `jsonPath`
 * check skips silently, so a `jsonPath` accepted that way leaves no
 * trace in the log to search for. Neither skip is an open door: the
 * `modPath` test runs between the two and returns 400 whenever the mods
 * path is unconfigured, so the request cannot start an install.
 *
 * ## :material-sync: Async lifecycle
 *
 * Both `handle_upload` and `handle_install` enqueue work on
 * `BackgroundJob<InstallJobResult> job_` and return 200 with
 * `{ "started": true, ... }` before the install runs. Clients poll
 * `handle_status` to observe completion.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * sequenceDiagram
 *     participant C as Client
 *     participant Ctl as InstallationController
 *     participant T as temp dir
 *     participant J as BackgroundJob
 *     C->>Ctl: POST /api/installation/upload
 *     Ctl->>Ctl: body over 8 GiB? then 413
 *     Ctl->>T: save archive as mo2_upload_[12 hex][ext]
 *     Ctl->>T: write fomodJson temp file (optional)
 *     Ctl->>Ctl: validate modName and modPath
 *     alt validation fails
 *       Ctl-->>C: 400 (temp files are left behind, see the known leak)
 *     else job slot busy
 *       Ctl->>T: remove temp files
 *       Ctl-->>C: 409
 *     else accepted
 *       Ctl->>J: try_start(work) - temp-file ownership moves to the job
 *       Ctl-->>C: 200 started
 *       J->>J: SalmaEngine::install_mod
 *       J->>T: remove temp files (success or failure)
 *     end
 *     loop poll
 *       C->>Ctl: GET /api/installation/status/current
 *       Ctl-->>C: 200 running / success / modPath / error
 *     end
 * ```
 *
 * ## :material-information-outline: Notes
 *
 * ### Which selections JSON gets used
 *
 * When the upload carries no `fomodJson` part, the controller searches
 * the configured FOMOD output directory for a saved selections file, in
 * this order:
 *
 * 1. Exact `<modName>.json`.
 * 2. Exact `<archive stem>.json`.
 * 3. Exact `<archive stem with the Nexus suffix stripped>.json`, where
 *    the stripped suffix is the `-<modId>-<version>-<fileId>` tail.
 * 4. A prefix match over every `.json` in that directory, longest stem
 *    first, case-insensitively. A candidate matches when its stem is a
 *    prefix of one of the three keys above, the stem is at least 5
 *    characters, the stem covers at least 50% of the key, and the next
 *    character in the key is `-`, `_`, space or dot.
 *
 * Step 4 is a heuristic and can attach a different mod's selections file
 * to this install when two mod names share a long prefix. Nothing in the
 * 200 response says which JSON was chosen; the log line
 * `[install] Using existing FOMOD JSON` is the only record.
 *
 * If the controller finds nothing, the engine applies its own sidecar
 * rule: a `.json` file next to the archive with the same stem. For an
 * upload, that rule can only ever match the temp JSON this same request
 * wrote, because the archive lives at a random temp path.
 *
 * ### Temp-file ownership
 *
 * Temp files created during upload (the archive and any caller-supplied
 * FOMOD JSON written to a temp path) are owned by the controller until
 * the background job starts, and by the job afterwards. Ownership moves
 * at exactly one line: the `try_start` call that returns true.
 *
 * | Exit path | Who deletes the temp files |
 * |-----------|----------------------------|
 * | 413 body over cap | nothing has been written yet |
 * | 400 raised inside `parse_and_validate_upload` | no one - see the leak below |
 * | 409 job slot busy | the controller, before returning |
 * | 500 exception | the controller's catch block |
 * | 200 job started | the background job, after the install finishes |
 *
 * **Known leak.** `parse_and_validate_upload` saves the archive first
 * and validates `modName` and `modPath` afterwards, and it returns a 400
 * rather than throwing. `handle_upload` returns on that path before it
 * records the paths to clean up, so every 400 raised after the archive
 * is written leaves an archive-sized temp file behind. Only the first
 * 400 (`No file uploaded or file is empty`) is safe, because nothing was
 * saved by then. The 409 and exception paths clean up correctly.
 *
 * ## :material-code-tags: Usage example
 *
 * ```cpp
 * InstallationController ctrl;
 * CROW_ROUTE(app, "/api/installation/upload").methods("POST"_method)(
 *     [&](const crow::request& req) { return ctrl.handle_upload(req); });
 * ```
 *
 * @see MultipartHandler, SalmaEngine, BackgroundJob
 */
class InstallationController
{
public:
    InstallationController() = default;
    InstallationController(const InstallationController&) = delete;
    InstallationController& operator=(const InstallationController&) = delete;
    /**
     * @brief Handle a multipart archive upload and install.
     *
     * Expects a multipart form with a `file` part (the archive) and
     * optional `modName`, `modPath` and `fomodJson` text parts. Saves the
     * file to a temp path, writes the FOMOD JSON if one was supplied, and
     * runs the install on the background thread (`job_`) through
     * `SalmaEngine::install_mod`.
     *
     * A body larger than the 8 GiB upload cap returns HTTP 413 before any
     * multipart parsing or temp-file write. The cap is the only size guard:
     * Crow bounds no request body, so `req.body` is already buffered in full
     * by the time this check runs, and the 413 saves the parse and the disk
     * write but not the memory. `main.cpp`'s `stream_threshold` holds the same
     * value but is unrelated; it bounds a response.
     *
     * **`modName` resolution.** The `modName` in the 200 response, also
     * used for the existing-FOMOD-JSON lookup, comes from:
     *
     * 1. The multipart `modName` field, if non-empty.
     * 2. Otherwise the upload filename's stem (`fs::path::stem`) with a
     *    leading `<digits>[-_]` prefix stripped, so a Nexus download
     *    named `12345-MyMod-v1.7z` becomes `MyMod-v1`.
     *
     * The result is checked with `mo2core::is_safe_mod_name`, so a name
     * carrying a path separator, a `..` segment or a reserved character
     * is rejected with 400.
     *
     * @param req The Crow HTTP request.
     * @return JSON response with install result or error. May return
     *         413 when the body exceeds the upload-size cap.
     * @note Catches `std::exception` and reports it as a 500 JSON error
     *       response. There is no `catch (...)` arm, so an exception
     *       that does not derive from `std::exception` escapes into
     *       Crow.
     */
    crow::response handle_upload(const crow::request& req);

    /**
     * @brief Install from an archive already on disk.
     *
     * Expects a JSON body with the required `archivePath` and `modPath`
     * fields, plus an optional `jsonPath` naming a FOMOD selections JSON
     * file. All three are checked for containment against the roots
     * listed in the class block before the job starts.
     *
     * Creates no temp file, so no exit path has anything to clean up. The
     * archive is read where it already sits.
     *
     * The job started here leaves `mod_name` empty, so a later
     * handle_status() reports no `modName` field.
     *
     * @param req The Crow HTTP request.
     * @return JSON response with install result or error.
     * @note Catches `std::exception` and reports it as a 500 JSON error
     *       response. There is no `catch (...)` arm, so an exception
     *       that does not derive from `std::exception` escapes into
     *       Crow.
     */
    crow::response handle_install(const crow::request& req);

    /**
     * @brief Read the running/completed state of the active install job.
     *
     * Reads `BackgroundJob<InstallJobResult>` state under its mutex and
     * returns a JSON status payload. Always 200: with one job slot there
     * is no "job not found" case to report.
     *
     * The slot is not cleared between installs, so after one completes
     * this keeps reporting that install's result until the next starts.
     *
     * @param job_id Soft-contract: the only meaningful value is `"current"`.
     *        Any other value is accepted (still returns 200) but a warning
     *        is logged to make stale-poller bugs visible.
     * @return 200 with `{ "running": bool, "success"?: bool, "modPath"?: ...,
     *         "modName"?: ..., "error"?: ... }`. Optional fields are present
     *         only after the job has stored a result (see the class shape
     *         table). The body is the bare `{ "running": false }` in two
     *         cases: before any install has ever been started, and after a
     *         worker died without storing a result, which is possible only
     *         for an exception that does not derive from `std::exception`.
     *         The two are indistinguishable from the response alone.
     * @note Does not throw. The body is read inside `read_result`'s
     *       mutex-held callback so `running` and the result fields are
     *       observed atomically.
     */
    crow::response handle_status(const std::string& job_id);

private:
    struct UploadContext
    {
        std::string temp_path;
        std::string filename;
        std::string mod_name;
        std::string mod_path;
        std::string json_path;
        bool json_is_temp = false;
    };

    std::optional<UploadContext> parse_and_validate_upload(const crow::request& req,
                                                           crow::response& error_out);

    BackgroundJob<InstallJobResult> job_;
};

}  // namespace mo2server

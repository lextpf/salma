#pragma once

#include <crow.h>
#include <nlohmann/json.hpp>

#include <optional>
#include <string>

#include "BackgroundJob.h"

namespace mo2server
{

/**
 * @struct InstallJobResult
 * @brief Result payload stored by the background installation job.
 * @author Alex (https://github.com/lextpf)
 *
 * Separate from mo2core::InstallResult because the background job also
 * tracks mod_name (derived from the upload filename or user input),
 * which is not part of the core install result.
 */
struct InstallJobResult
{
    bool success = false;
    std::string mod_path;
    std::string mod_name;
    std::string error;
};

/**
 * @class InstallationController
 * @brief REST endpoints for upload, install, and status.
 * @author Alex (https://github.com/lextpf)
 * @ingroup InstallationController
 *
 * Handles HTTP requests from the React frontend for the server-based
 * installation workflow. Parses multipart uploads via MultipartHandler,
 * delegates installation to InstallationService, and returns JSON
 * responses.
 *
 * ## :material-api: Endpoints
 *
 * | Method |              Route              | Handler                                          |
 * |--------|---------------------------------|--------------------------------------------------|
 * |   POST |   `/api/installation/upload`    | handle_upload - receive archive + install        |
 * |   POST |   `/api/installation/install`   | handle_install - install from existing path      |
 * |    GET | `/api/installation/status/<id>` | handle_status - stub, always returns "completed" |
 *
 * ## :material-code-braces: Response Shapes
 *
 * | Route              | Status  | Body                                                        |
 * |--------------------|---------|-------------------------------------------------------------|
 * | `POST .../upload`  | **200** | `{ "started": true, "modName": "..." }`                     |
 * |                    | **400** | `{ "error": "No file uploaded or file is empty" }`          |
 * |                    | **400** | `{ "error": "No modPath provided and MO2 mods path is..." }`|
 * |                    | **409** | `{ "error": "An installation is already running" }`         |
 * |                    | **413** | `{ "error": "Upload exceeds 512 MiB limit" }`               |
 * | `POST .../install` | **200** | `{ "started": true }`                                       |
 * |                    | **400** | `{ "error": "ArchivePath and ModPath are required" }`       |
 * |                    | **409** | `{ "error": "An installation is already running" }`         |
 * | `GET .../status/x` | **200** | `{ "running": bool, "success"?: bool, "modPath"?: "...",    |
 * |                    |         |   "modName"?: "...", "error"?: "..." }`                     |
 * | *(any)*            | **500** | `{ "error": "..." }`                                        |
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * InstallationController ctrl;
 * CROW_ROUTE(app, "/api/installation/upload").methods("POST"_method)(
 *     [&](const crow::request& req) { return ctrl.handle_upload(req); });
 * ```
 *
 * ## :material-information-outline: Notes
 *
 * - If no `fomodJson` is provided in the upload and a `.json` file
 *   exists adjacent to the archive (same stem), it is used
 *   automatically by InstallationService.
 * - Temp files created during upload (archive and any caller-supplied
 *   FOMOD JSON written to a temp path) are cleaned up by the
 *   background job after the install completes (success or failure).
 *   If the job fails to start (409) or `parse_and_validate_upload`
 *   throws, the controller deletes any already-saved temp files
 *   before returning, so the temp directory does not accumulate
 *   orphans.
 *
 * ## Async Lifecycle
 *
 * Both `handle_upload` and `handle_install` enqueue work on
 * `BackgroundJob<InstallJobResult> job_` and return 200 with
 * `{ "started": true, ... }` before the install runs. Clients poll
 * `handle_status` to observe completion.
 *
 * @see MultipartHandler, InstallationService, BackgroundJob
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
     * optional `modName`, `modPath`, and `fomodJson` text parts.
     * Saves the file to a temp path, writes the FOMOD JSON if
     * provided, and runs InstallationService::install_mod() on a
     * background thread (`job_`).
     *
     * Enforces a hard upload-size cap of **512 MiB**: requests with a
     * body larger than this return HTTP 413 before any multipart
     * parsing or temp-file write.
     *
     * ### `modName` resolution
     *
     * The `modName` field returned in the 200 response and used for
     * existing-FOMOD-JSON lookup is determined as:
     *
     * 1. Multipart `modName` field, if non-empty.
     * 2. Otherwise, the upload filename's stem (`fs::path::stem`)
     *    with a leading `<digits>[-_]` prefix stripped (so a Nexus
     *    download named `12345-MyMod-v1.7z` becomes `MyMod-v1`).
     *
     * @param req The Crow HTTP request.
     * @return JSON response with install result or error. May return
     *         413 when the body exceeds the upload-size cap.
     * @note Does not throw. All exceptions are caught internally and
     *       returned as a 500 JSON error response.
     */
    crow::response handle_upload(const crow::request& req);

    /**
     * @brief Install from an archive already on disk.
     *
     * Expects a JSON body with `archivePath` and `modPath` fields
     * (required), and an optional `jsonPath` field pointing to a
     * FOMOD selections JSON file.
     *
     * @param req The Crow HTTP request.
     * @return JSON response with install result or error.
     * @note Does not throw. All exceptions are caught internally and
     *       returned as a 500 JSON error response.
     */
    crow::response handle_install(const crow::request& req);

    /**
     * @brief Read the running/completed state of the active install job.
     *
     * Reads `BackgroundJob<InstallJobResult>` state under its mutex and
     * returns a JSON status payload. Always returns 200 -- there is no
     * "job not found" error because the controller only tracks one slot.
     *
     * @param job_id Soft-contract: the only meaningful value is `"current"`.
     *        Any other value is accepted (still returns 200) but a warning
     *        is logged to make stale-poller bugs visible.
     * @return 200 with `{ "running": bool, "success"?: bool, "modPath"?: ...,
     *         "modName"?: ..., "error"?: ... }`. Optional fields are present
     *         only after the job has completed (see the class shape table).
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

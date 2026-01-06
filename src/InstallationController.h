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
 * - Temp files created during upload (archive and JSON) are cleaned
 *   up after a **successful** installation. If installation throws,
 *   the error path does not delete the temp files -- they remain on
 *   disk until the OS cleans the temp directory.
 *
 * @see MultipartHandler, InstallationService
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
     * provided, and runs InstallationService::install_mod().
     *
     * @param req The Crow HTTP request.
     * @return JSON response with install result or error.
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
     * @brief Check the status of an installation job (stub).
     *
     * Currently always returns `{"status": "completed"}`. No real job
     * tracking is implemented - all installs run synchronously.
     *
     * @param job_id The job identifier (unused).
     * @return JSON response with `"completed"` status.
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

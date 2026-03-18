#pragma once

#include <crow.h>
#include <nlohmann/json.hpp>

#include <atomic>
#include <mutex>
#include <string>
#include <thread>

namespace mo2server
{

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
 * | `POST .../upload`  | **200** | `{ "success": true, "modPath": "...", "modName": "..." }`   |
 * |                    | **400** | `{ "error": "No file uploaded or file is empty" }`          |
 * |                    | **400** | `{ "error": "No modPath provided and MO2 mods path is..." }`|
 * | `POST .../install` | **200** | `{ "success": true, "outputPath": "..." }`                  |
 * |                    | **400** | `{ "error": "ArchivePath and ModPath are required" }`       |
 * | `GET .../status/x` | **200** | `{ "status": "completed", "jobId": "..." }`                 |
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
    ~InstallationController()
    {
        if (install_thread_.joinable())
        {
            install_thread_.join();
        }
    }
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
    std::mutex install_mutex_;
    std::atomic<bool> install_running_{false};
    bool install_has_result_{false};
    bool install_last_success_{false};
    std::string install_result_mod_path_;
    std::string install_result_mod_name_;
    std::string install_last_error_;
    std::thread install_thread_;
};

}  // namespace mo2server

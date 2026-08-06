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
 * @brief Stores the outcome of one background installation.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup InstallationController
 *
 * Success sets `mod_path`. Failure sets `error`. Upload jobs also set `mod_name`.
 */
struct InstallJobResult
{
    bool success = false;  ///< True after a completed engine install.
    std::string mod_path;  ///< Path reported by the engine.
    std::string mod_name;  ///< Resolved upload name, or empty for direct installs.
    std::string error;     ///< Engine failure text, or empty on success.
};

/**
 * @class InstallationController
 * @brief Accepts archives and runs one installation job at a time.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup InstallationController
 *
 * Uploads and direct installs share one `BackgroundJob` slot. Accepted requests
 * start work asynchronously; clients poll the status endpoint. The engine call
 * does not poll the job cancellation token.
 *
 * ### :material-shield-lock: Security invariants
 *
 * | Input         | Required containment root                               |
 * |---------------|---------------------------------------------------------|
 * | `archivePath` | Configured mods or `SALMA_DOWNLOADS_PATH`               |
 * | `modPath`     | Configured mods                                         |
 * | `jsonPath`    | Mods, downloads, FOMOD output, or the archive directory |
 *
 * `SALMA_DOWNLOADS_PATH` is used only when it is absolute and not a root path.
 * Upload bodies are limited to 8 GiB, but Crow buffers the body before this check.
 *
 * ### :material-link-variant: Selections lookup
 *
 * When no selections part is supplied, lookup checks exact mod and archive names,
 * then a constrained case-insensitive prefix match. A prefix match can select the
 * wrong file when mods share a long prefix.
 *
 * ### :material-state-machine: Job lifecycle
 *
 * ```mermaid
 * flowchart TD
 *     request --> validate[validate input and containment]
 *     validate -->|reject| error[return error]
 *     validate --> slot{job slot free?}
 *     slot -->|no| cleanup[remove owned temporary files]
 *     slot -->|yes| engine[run engine in worker]
 *     engine --> cleanup
 *     cleanup --> status[publish result]
 * ```
 *
 * @warning Validation failures after an upload is saved can leave temporary
 *          files behind. Cleanup ownership transfers only after validation succeeds.
 *
 * @see MultipartHandler, SalmaEngine, BackgroundJob
 */
class InstallationController
{
public:
    /**
     * @fn InstallationController::InstallationController()
     * @brief Initialize an idle installation slot.
     * @author Alex (<https://github.com/lextpf>)
     */
    InstallationController() = default;
    InstallationController(const InstallationController&) = delete;
    InstallationController& operator=(const InstallationController&) = delete;
    /**
     * @fn crow::response InstallationController::handle_upload(const crow::request&)
     * @brief Applies the 8 GiB limit before multipart parsing.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Expects a `file` part and optional `modName`, `modPath`, and `fomodJson`
     * fields. Unsafe mod names are rejected. HTTP 413 is returned before parsing
     * or writing when the buffered body exceeds 8 GiB.
     *
     * @param req Multipart request.
     * @return HTTP 200 after start, 400 for invalid input, 409 when busy, 413 for
     *         an oversized body, or 500 for a standard exception.
     */
    crow::response handle_upload(const crow::request& req);

    /**
     * @fn crow::response InstallationController::handle_install(const crow::request&)
     * @brief Accepts only paths contained by configured roots.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Expects `archivePath` and `modPath`, with optional `jsonPath`. All paths use
     * the listed containment rules. This route creates no temporary files.
     *
     * @param req JSON request.
     * @return HTTP 200 after start, 400 for invalid input, 409 when busy, or 500
     *         for a standard exception.
     */
    crow::response handle_install(const crow::request& req);

    /**
     * @fn crow::response InstallationController::handle_status(const std::string&)
     * @brief Reports one synchronized snapshot for any job identifier.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The result remains until another installation starts. Any job identifier
     * is accepted, but values other than `current` are logged.
     *
     * @param job_id Expected value is `current`.
     * @return HTTP 200 with one synchronized job snapshot.
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

    /**
     * @fn std::optional<UploadContext> parse_and_validate_upload(const crow::request&,
     *     crow::response&)
     * @brief Save the upload and resolve validated installation inputs.
     * @author Alex (<https://github.com/lextpf>)
     *
     * On success the caller owns the archive and any selections file marked temporary. Failure
     * after a write can leave those files on disk.
     *
     * @param req Buffered multipart request, limited to 8 GiB before parsing.
     * @param error_out Receives the HTTP error response when no context is returned.
     * @return Validated inputs, or no value after a rejected request.
     */
    std::optional<UploadContext> parse_and_validate_upload(const crow::request& req,
                                                           crow::response& error_out);

    BackgroundJob<InstallJobResult> job_;
};

}  // namespace mo2server

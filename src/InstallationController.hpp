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
 * @brief stores the outcome of one background installation.
 * @author Alex (https://github.com/lextpf)
 * @ingroup InstallationController
 *
 * success sets `mod_path`. failure sets `error`. upload jobs also set `mod_name`.
 */
struct InstallJobResult
{
    bool success = false;  ///< true after a completed engine install.
    std::string mod_path;  ///< path reported by the engine.
    std::string mod_name;  ///< resolved upload name, or empty for direct installs.
    std::string error;     ///< engine failure text, or empty on success.
};

/**
 * @class InstallationController
 * @brief accepts archives and runs one installation job at a time.
 * @author Alex (https://github.com/lextpf)
 * @ingroup InstallationController
 *
 * uploads and direct installs share one `BackgroundJob` slot. accepted requests
 * return before the engine runs; clients poll the status endpoint.
 *
 * ### :material-shield-lock: security invariants
 *
 * | input         | required containment root                                  |
 * |---------------|------------------------------------------------------------|
 * | `archivePath` | configured mods or `SALMA_DOWNLOADS_PATH`                  |
 * | `modPath`     | configured mods                                            |
 * | `jsonPath`    | mods, downloads, FOMOD output, or the archive directory    |
 *
 * `SALMA_DOWNLOADS_PATH` is used only when it is absolute and not a root path.
 * upload bodies are limited to 8 GiB, but Crow buffers the body before this check.
 *
 * ### :material-link-variant: selections lookup
 *
 * when no selections part is supplied, lookup checks exact mod and archive names,
 * then a constrained case-insensitive prefix match. a prefix match can select the
 * wrong file when mods share a long prefix.
 *
 * ### :material-state-machine: job lifecycle
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
 * @warning validation failures after an upload is saved can leave the temporary
 *          archive behind. busy and exception paths remove their files.
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
     * @fn crow::response InstallationController::handle_upload(const crow::request&)
     * @brief applies the 8 GiB limit before multipart parsing.
     * @author Alex (https://github.com/lextpf)
     *
     * expects a `file` part and optional `modName`, `modPath`, and `fomodJson`
     * fields. names are reduced to one safe directory component. HTTP 413 is
     * returned before parsing or writing when the buffered body exceeds 8 GiB.
     *
     * @param req multipart request.
     * @return HTTP 200 after start, 400 for invalid input, 409 when busy, 413 for
     *         an oversized body, or 500 for a standard exception.
     */
    crow::response handle_upload(const crow::request& req);

    /**
     * @fn crow::response InstallationController::handle_install(const crow::request&)
     * @brief accepts only paths contained by configured roots.
     * @author Alex (https://github.com/lextpf)
     *
     * expects `archivePath` and `modPath`, with optional `jsonPath`. all paths use
     * the listed containment rules. this route creates no temporary files.
     *
     * @param req JSON request.
     * @return HTTP 200 after start, 400 for invalid input, 409 when busy, or 500
     *         for a standard exception.
     */
    crow::response handle_install(const crow::request& req);

    /**
     * @fn crow::response InstallationController::handle_status(const std::string&)
     * @brief reports one synchronized snapshot for any job identifier.
     * @author Alex (https://github.com/lextpf)
     *
     * the result remains until another installation starts. any job identifier
     * is accepted, but values other than `current` are logged.
     *
     * @param job_id expected value is `current`.
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

    std::optional<UploadContext> parse_and_validate_upload(const crow::request& req,
                                                           crow::response& error_out);

    BackgroundJob<InstallJobResult> job_;
};

}  // namespace mo2server

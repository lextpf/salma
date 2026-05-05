/*  ============================================================================================  *
 *
 *       ::::::::      :::     :::        ::::    ::::      :::         ⢠⣤⣤⣀ ⠀⠀⠀⠀⠀⠀ ⣀⣤⣤⡄
 *      :+:    :+:   :+: :+:   :+:        +:+:+: :+:+:+   :+: :+:      ⢸⣿⣿⣿⣿⣦⣄⣀⣠⣴⣿⣿⣿⣿⡇⠀*
 *      +:+         +:+   +:+  +:+        +:+ +:+:+ +:+  +:+   +:+     ⣸⣿⣿⣿⣿⣿⡽⣿⣯⣿⣿⣿⣿⣿⣇
 *      +#++:++#++ +#++:++#++: +#+        +#+  +:+  +#+ +#++:++#++:    ⢻⣿⣿⣿⠿⣻⣵⡟⣮⣟⠿⣿⣿⣿⡟
 *             +#+ +#+     +#+ +#+        +#+       +#+ +#+     +#+    ⠀⠀⠀⠀⣼⣿⡿ ⠀⢿⣿⣷⡀
 *      #+#    #+# #+#     #+# #+#        #+#       #+# #+#     #+#    *⠀⣠⣾⣿⣿⠃ ⠀⠈⢿⣿⣿⣦⡀
 *       ########  ###     ### ########## ###       ### ###     ###    ⠀⠈⠉⠹⡿⠁⠀⠀⠀⠀⠈⢻⡇⠉⠉
 *
 *                              << F O M O D   E N G I N E >>
 *
 *  ============================================================================================  *
 *
 *      A Crow HTTP server that hosts the React frontend and exposes
 *      REST endpoints for wizardless FOMOD processing, install replay,
 *      and inference,
 * backed by libarchive, bit7z, and pugixml.
 *
 *    ----------------------------------------------------------------------
 *
 *      Repository:   https://github.com/lextpf/salma
 *      License:      MIT
 */
#include <crow.h>

#include "ConfigService.h"
#include "InstallationController.h"
#include "Logger.h"
#include "Mo2Controller.h"
#include "SecurityContext.h"
#include "SecurityMiddleware.h"
#include "StaticFileHandler.h"
#include "Utils.h"

#include <crow/logging.h>
#include <cstdlib>
#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>
#include <string>

namespace fs = std::filesystem;

/**
 * @brief Bridges Crow's ILogHandler into salma's Logger so all HTTP-server
 *        output shares the same timestamp + level format as the rest of salma.
 *
 * **Suppression policy.**  Heartbeat endpoints (`/api/logs`,
 * `/api/logs/test`, `/api/mo2/status`) generate constant polling traffic
 * that would drown out useful log lines.  Their Request and Response entries
 * are silently dropped -- except for Response lines with a non-200 status,
 * which are kept for debugging.
 *
 * **Routing rules.**  Crow log levels are mapped to salma Logger calls:
 * - `Error` / `Critical` -> Logger::log_error
 * - `Warning`            -> Logger::log_warning
 * - everything else      -> Logger::log
 *
 * Every forwarded message is prefixed with `[crow]`.
 *
 * @note Suppression uses string-prefix matching (`.starts_with()`), so any
 *       non-Crow message that happens to start with "Request:" or "Response:"
 *       would also be filtered.  In practice only Crow produces these
 *       prefixes, so this is not an issue.
 *
 * **Extending suppression.**  To suppress additional endpoints, add a
 * new `message.find(...)` clause to the `is_heartbeat` check in
 * should_suppress_noise(). Test by temporarily enabling verbose Crow
 * logging (LogLevel::Debug) and verifying the target lines no longer
 * appear in salma.log while non-200 responses still do.
 */
class SalmaLogHandler : public crow::ILogHandler
{
public:
    static bool should_suppress_noise(const std::string& message)
    {
        const bool is_request = message.starts_with("Request:");
        const bool is_response = message.starts_with("Response:");
        if (!is_request && !is_response)
        {
            return false;
        }

        const bool is_heartbeat = message.find("GET /api/logs") != std::string::npos ||
                                  message.find("/api/logs?") != std::string::npos ||
                                  message.find("GET /api/logs/test") != std::string::npos ||
                                  message.find("/api/logs/test?") != std::string::npos ||
                                  message.find("GET /api/mo2/status") != std::string::npos ||
                                  message.find("/api/mo2/status ") != std::string::npos;
        if (!is_heartbeat)
        {
            return false;
        }

        // Keep failed polling responses for debugging, but suppress the
        // expected heartbeat traffic.
        if (is_response)
        {
            return message.find(" 200 ") != std::string::npos;
        }
        return true;
    }

    void log(const std::string& message, crow::LogLevel level) override
    {
        if (should_suppress_noise(message))
        {
            return;
        }

        auto& logger = mo2core::Logger::instance();
        switch (level)
        {
            case crow::LogLevel::Error:
            case crow::LogLevel::Critical:
                logger.log_error(std::format("[crow] {}", message));
                break;
            case crow::LogLevel::Warning:
                logger.log_warning(std::format("[crow] {}", message));
                break;
            default:
                logger.log(std::format("[crow] {}", message));
                break;
        }
    }
};

int main()
{
    auto& logger = mo2core::Logger::instance();

    static SalmaLogHandler crow_log_handler;
    crow::logger::setHandler(&crow_log_handler);

    logger.log("[server] Starting server...");

    mo2server::ConfigService::instance().load();

    // Touch the SecurityContext singleton up-front so the CSRF token is
    // generated and the Origin allowlist is parsed before the first
    // request can race against a still-uninitialized state.
    auto& security = mo2core::SecurityContext::instance();
    {
        std::string joined;
        for (const auto& origin : security.allowed_origins())
        {
            if (!joined.empty())
            {
                joined += ", ";
            }
            joined += origin;
        }
        logger.log("[server] CSRF token generated (64 hex chars)");
        logger.log(std::format("[server] Allowed origins: {}", joined));
    }

    // CORS + CSRF policy is enforced by SecurityMiddleware. See SecurityMiddleware.h.
    crow::App<mo2server::SecurityMiddleware> app;

    mo2server::InstallationController controller;
    mo2server::Mo2Controller mo2_controller;

    // Static file directory: anchor against the exe location, not cwd, so the
    // dashboard works regardless of where the user launches mo2-server.exe from.
    // Try <exe>/web/dist first (release layout), fall back to <exe>/../web/dist
    // (in-tree dev layout where the exe lives in build/bin/Release).
    auto exe_dir = mo2core::executable_directory();
    auto static_dir = (exe_dir / "web" / "dist").string();
    if (!fs::exists(static_dir))
    {
        static_dir = (exe_dir.parent_path() / "web" / "dist").string();
    }
    if (!fs::exists(static_dir))
    {
        logger.log_warning(
            std::format("[server] Static files directory not found: {}", static_dir));
    }
    logger.log(std::format("[server] Static files directory: {}", static_dir));
    mo2server::StaticFileHandler static_handler(static_dir);

    // POST /api/installation/upload    - multipart archive upload + install
    // POST /api/installation/install   - install from existing archive path
    // GET  /api/installation/status/id - check job status
    CROW_ROUTE(app, "/api/installation/upload")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_upload(req); });

    CROW_ROUTE(app, "/api/installation/install")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_install(req); });

    CROW_ROUTE(app, "/api/installation/status/<string>")
        .methods(crow::HTTPMethod::GET)([&controller](const std::string& job_id)
                                        { return controller.handle_status(job_id); });

    // CSRF token endpoint. SecurityMiddleware gates state-changing requests
    // by an X-Salma-Csrf header that must match this token. The token is
    // readable only from allowlisted origins (CORS), so cross-origin
    // attackers cannot fetch it to forge requests.
    CROW_ROUTE(app, "/api/csrf-token")
        .methods(crow::HTTPMethod::GET)(
            [&security]()
            {
                nlohmann::json body = {{"token", security.csrf_token()}};
                crow::response res(200, body.dump());
                res.set_header("Content-Type", "application/json");
                res.set_header("Cache-Control", "no-store");
                return res;
            });

    // MO2 integration routes
    CROW_ROUTE(app, "/api/config")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.get_config(); });

    CROW_ROUTE(app, "/api/config")
        .methods(crow::HTTPMethod::PUT)([&mo2_controller](const crow::request& req)
                                        { return mo2_controller.put_config(req); });

    CROW_ROUTE(app, "/api/mo2/status")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.get_status(); });

    CROW_ROUTE(app, "/api/mo2/fomods")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.list_fomods(); });

    CROW_ROUTE(app, "/api/mo2/fomods/scan")
        .methods(crow::HTTPMethod::POST)([&mo2_controller]()
                                         { return mo2_controller.scan_fomods(); });

    CROW_ROUTE(app, "/api/mo2/fomods/scan/status")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.get_scan_status(); });

    CROW_ROUTE(app, "/api/mo2/fomods/<string>")
        .methods(crow::HTTPMethod::GET)([&mo2_controller](const std::string& name)
                                        { return mo2_controller.get_fomod(name); });

    CROW_ROUTE(app, "/api/mo2/fomods/<string>")
        .methods("DELETE"_method)([&mo2_controller](const std::string& name)
                                  { return mo2_controller.delete_fomod(name); });

    CROW_ROUTE(app, "/api/plugin/deploy")
        .methods(crow::HTTPMethod::POST)([&mo2_controller]()
                                         { return mo2_controller.deploy_plugin(); });

    CROW_ROUTE(app, "/api/plugin/purge")
        .methods(crow::HTTPMethod::POST)([&mo2_controller]()
                                         { return mo2_controller.purge_plugin(); });

    CROW_ROUTE(app, "/api/plugin/status")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.get_plugin_action_status(); });

    CROW_ROUTE(app, "/api/logs")
        .methods(crow::HTTPMethod::GET)([&mo2_controller](const crow::request& req)
                                        { return mo2_controller.get_logs(req); });

    CROW_ROUTE(app, "/api/logs/test")
        .methods(crow::HTTPMethod::GET)([&mo2_controller](const crow::request& req)
                                        { return mo2_controller.get_test_logs(req); });

    CROW_ROUTE(app, "/api/logs/clear")
        .methods(crow::HTTPMethod::POST)([&mo2_controller]()
                                         { return mo2_controller.clear_logs(); });

    CROW_ROUTE(app, "/api/logs/clear/test")
        .methods(crow::HTTPMethod::POST)([&mo2_controller]()
                                         { return mo2_controller.clear_test_logs(); });

    CROW_ROUTE(app, "/api/test/run")
        .methods(crow::HTTPMethod::POST)([&mo2_controller](const crow::request& req)
                                         { return mo2_controller.run_tests(req); });

    CROW_ROUTE(app, "/api/test/status")
        .methods(crow::HTTPMethod::GET)([&mo2_controller]()
                                        { return mo2_controller.get_test_status(); });

    // Non-API paths are served from the static directory.
    // Unknown paths fall through to index.html for client-side routing.
    CROW_ROUTE(app, "/")
    ([&static_handler]() { return static_handler.serve(""); });

    CROW_ROUTE(app, "/<path>")
    (
        [&static_handler](const std::string& path)
        {
            if (path.substr(0, 4) == "api/")
            {
                return crow::response(404);
            }
            return static_handler.serve(path);
        });

    // stream_threshold controls when Crow starts streaming request bodies instead
    // of buffering them in memory. It does NOT enforce a hard body-size limit.
    // The actual upload-size cap is enforced per-handler (see handle_upload's
    // kMaxUploadBytes in InstallationController.cpp - same 512 MiB value, by design).
    static constexpr size_t kStreamThreshold = 512ULL * 1024 * 1024;

    // Bind to loopback by default. The server exposes endpoints that perform
    // file writes, archive extraction, batch script execution, and child-process
    // spawning; opening that surface to the LAN is unsafe. Users who deliberately
    // need a non-loopback bind (e.g. dev container, remote dashboard) can set
    // SALMA_BIND_ADDR -- and the warning makes that choice visible in the log.
    std::string bind_addr = "127.0.0.1";
    if (const char* bind_env = std::getenv("SALMA_BIND_ADDR"); bind_env && *bind_env)
    {
        bind_addr = bind_env;
    }
    if (bind_addr != "127.0.0.1" && bind_addr != "localhost" && bind_addr != "::1")
    {
        logger.log_warning(
            std::format("[server] SALMA_BIND_ADDR set to non-loopback address {}; "
                        "endpoints that write files and spawn processes are now reachable "
                        "from the network. Ensure the host firewall is configured.",
                        bind_addr));
    }

    logger.log(std::format("[server] Server starting on {}:5000", bind_addr));
    app.bindaddr(bind_addr).port(5000).multithreaded().stream_threshold(kStreamThreshold).run();

    return 0;
}

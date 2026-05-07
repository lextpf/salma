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
 *      and inference. The server itself does no archive, XML or FOMOD
 *      work: it links only Crow and nlohmann-json, and forwards every
 *      install, inference and archive-resolution call to the Rust engine
 *      mo2-salma.dll through src/SalmaEngine.cpp.
 *
 *    ----------------------------------------------------------------------
 *
 *      Repository:   https://github.com/lextpf/salma
 *      License:      MIT
 */
#include <crow.h>

#include "ConfigService.hpp"
#include "InstallationController.hpp"
#include "Logger.hpp"
#include "Mo2Controller.hpp"
#include "SecurityContext.hpp"
#include "SecurityMiddleware.hpp"
#include "StaticFileHandler.hpp"
#include "Utils.hpp"

#include <crow/logging.h>
#include <cstdlib>
#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>
#include <string>

namespace fs = std::filesystem;

// main - process entry point and the server's route table.
//
// Start-up runs in this order, and each step depends on the one before it:
//   1. Install SalmaLogHandler, so Crow's own output joins salma.log from the
//      first line rather than going to Crow's default sink.
//   2. ConfigService::load() reads salma.json next to the executable.
//   3. Touch the SecurityContext singleton, so the CSRF token and the Origin
//      allowlist exist before any request can observe them.
//   4. Construct crow::App<SecurityMiddleware>, the controllers and the static
//      file handler, register every route, then bind and run.
//
// The engine DLL is not loaded here. SalmaEngine loads mo2-salma.dll lazily on
// the first install, inference or archive-resolution call, so the dashboard
// still starts and serves pages when the engine is missing.
//
// Routes fall into three groups: /api/installation/*, the MO2 integration
// endpoints (/api/config, /api/mo2/*, /api/plugin/*, /api/logs*, /api/test/*),
// and every other path, which StaticFileHandler serves from web/dist with an
// index.html fallback for client-side routing. A path that starts with "api/"
// and matched no route returns 404 instead of falling through to index.html.
//
// app.run() blocks until Crow stops the app. The controllers are stack objects
// in main, so their destructors, which shut the background jobs down, run only
// after that return.

// Bridges Crow's ILogHandler into salma's Logger, so HTTP-server output shares
// the timestamp and level format of the rest of salma. Every forwarded message
// is prefixed with `[crow]`. Level mapping:
//   Error / Critical -> Logger::log_error
//   Warning          -> Logger::log_warning
//   everything else  -> Logger::log
//
// Suppression policy. The dashboard polls `/api/logs`, `/api/logs/test` and
// `/api/mo2/status` continuously. Their Request and Response lines would drown
// out everything else, so both are dropped. A Response line with a non-200
// status survives, because a failing poll is worth seeing.
//
// The filter matches a `starts_with` on "Request:" or "Response:", so a
// non-Crow message with either prefix would also be dropped. Only Crow
// produces them today.
//
// To suppress another endpoint, add a `message.find(...)` clause to the
// `is_heartbeat` check in should_suppress_noise(), then raise Crow to
// LogLevel::Debug and confirm the target lines stop reaching salma.log while
// non-200 responses still arrive.
//
// Keep this class on plain `//` comments: doxide globs src/*.cpp, and a block
// doc comment here would publish a page for an internal main.cpp helper that
// mkdocs.yml's nav never references.
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

        // Keep failed polls, drop the successful heartbeat traffic.
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

    // Touch the singleton before any route exists, so the CSRF token is
    // generated and the Origin allowlist parsed before a request can race an
    // uninitialized state.
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

    // SecurityMiddleware enforces the CORS and CSRF policy. See SecurityMiddleware.hpp.
    crow::App<mo2server::SecurityMiddleware> app;

    mo2server::InstallationController controller;
    mo2server::Mo2Controller mo2_controller;

    // Anchor the static directory to the exe location, not the working
    // directory, so the dashboard works wherever mo2-server.exe is launched
    // from. <exe>/web/dist is the release layout; <exe>/../web/dist is the
    // in-tree dev layout, where the exe sits in build/bin/Release.
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

    // POST /api/installation/upload         - multipart archive upload + install
    // POST /api/installation/install        - install from an existing archive path
    // GET  /api/installation/status/current - status of the single in-flight install
    //
    // The status path segment is ignored. InstallationController holds a single
    // BackgroundJob, not a map of jobs, so any segment other than "current"
    // logs a warning and still reports that one job. Do not build per-job
    // polling on this route.
    CROW_ROUTE(app, "/api/installation/upload")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_upload(req); });

    CROW_ROUTE(app, "/api/installation/install")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_install(req); });

    CROW_ROUTE(app, "/api/installation/status/<string>")
        .methods(crow::HTTPMethod::GET)([&controller](const std::string& job_id)
                                        { return controller.handle_status(job_id); });

    // SecurityMiddleware gates state-changing requests on an X-Salma-Csrf
    // header matching this token. CORS keeps the token readable only from
    // allowlisted origins, so a cross-origin attacker cannot fetch it and
    // forge a request.
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

    // stream_threshold bounds a response, not a request. It sets the res.body
    // size beyond which Crow streams instead of buffering: crow/app.h stores it
    // as res_stream_threshold_, and crow/http_connection.h compares it against
    // res.body on the write path. Set this high, Crow buffers every response.
    //
    // Nothing in Crow 1.3.0 bounds a request body. Its max_payload setting is
    // websocket-only, so req.body is buffered whole before any handler runs.
    // The upload cap is enforced only by kMaxUploadBytes, in
    // InstallationController::parse_and_validate_upload, and that check reads a
    // body that is already resident in memory. The two constants hold the same
    // value, but neither one constrains the other.
    //
    // The cost is real: the dashboard upload path holds the whole archive in
    // memory, once in req.body and again in the multipart copy, so a
    // multi-gigabyte drop needs multi-gigabyte headroom. The MO2 plugin has no
    // such limit, because it passes a path to the engine and uploads nothing.
    static constexpr size_t kStreamThreshold = 8ULL * 1024 * 1024 * 1024;

    // Bind to loopback by default. These endpoints write files, extract
    // archives, run batch scripts and spawn child processes, so exposing them
    // to the LAN is unsafe. SALMA_BIND_ADDR overrides the bind for cases that
    // need it, such as a dev container or a remote dashboard, and the warning
    // below makes that choice visible in the log.
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

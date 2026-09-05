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
 *      License:      GPL
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

// route Crow output through the shared logger and suppress successful poll noise.
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

        // retain failed polls and drop successful heartbeat traffic.
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

    // initialize the CSRF token and origin allowlist before routes become visible.
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

    // apply the CORS and CSRF policy to every route.
    crow::App<mo2server::SecurityMiddleware> app;

    mo2server::InstallationController controller;
    mo2server::Mo2Controller mo2_controller;

    // resolve release and development assets relative to the executable.
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

    // the status segment is advisory; every value reports the single install job.
    CROW_ROUTE(app, "/api/installation/upload")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_upload(req); });

    CROW_ROUTE(app, "/api/installation/install")
        .methods(crow::HTTPMethod::POST)([&controller](const crow::request& req)
                                         { return controller.handle_install(req); });

    CROW_ROUTE(app, "/api/installation/status/<string>")
        .methods(crow::HTTPMethod::GET)([&controller](const std::string& job_id)
                                        { return controller.handle_status(job_id); });

    // only allowed origins can read the token required for state-changing requests.
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

    // serve non-API paths with the client-side routing fallback.
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

    // this threshold controls response streaming. Crow buffers request bodies
    // before handlers run, so the 8 GiB upload check does not cap memory use.
    static constexpr size_t kStreamThreshold = 8ULL * 1024 * 1024 * 1024;

    // default to loopback because routes write files and start child processes.
    // SALMA_BIND_ADDR permits remote access and logs an explicit warning.
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

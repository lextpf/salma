#include "Mo2Controller.h"
#include "Mo2Helpers.h"

#include "Logger.h"
#include "Utils.h"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>
#include <regex>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// ---------------------------------------------------------------------------
// POST /api/test/run   -- spawn test.py in background
// ---------------------------------------------------------------------------

crow::response Mo2Controller::run_tests(const crow::request& req)
{
#ifdef _WIN32
    std::unique_lock<std::mutex> lock(test_mutex_);

    // Check if already running
    if (test_running_)
    {
        if (test_process_)
        {
            // Use WaitForSingleObject instead of GetExitCodeProcess+STILL_ACTIVE
            // to avoid the well-known Win32 pitfall where exit code 259 is
            // indistinguishable from a still-running process.
            DWORD wait_result = WaitForSingleObject(test_process_, 0);
            if (wait_result == WAIT_TIMEOUT)
            {
                return json_response(409, {{"error", "Tests are already running"}});
            }
            if (wait_result == WAIT_FAILED)
            {
                mo2core::Logger::instance().log_warning(std::format(
                    "[server] WaitForSingleObject failed (error {}), cleaning up", GetLastError()));
            }
            // Process finished or wait failed -- clean up
            CloseHandle(test_process_);
            test_process_ = nullptr;
            test_running_ = false;
        }
    }

    // Parse optional arguments from request body
    std::string args;
    if (!req.body.empty())
    {
        try
        {
            auto body = json::parse(req.body);
            if (body.contains("args") && body["args"].is_string())
                args = body["args"].get<std::string>();
        }
        catch (const std::exception& ex)
        {
            mo2core::Logger::instance().log_warning(
                std::format("[server] Invalid JSON in test request body: {}", ex.what()));
        }
        catch (...)
        {
            mo2core::Logger::instance().log_warning("[server] Invalid test request body");
        }
    }

    // Sanitize args: whitelist approach to prevent command injection.
    // Only alphanumeric, space, underscore, hyphen, and dot are allowed.
    // Quotes and path separators are deliberately excluded to prevent
    // argument-boundary escape and path traversal.
    static const std::regex kAllowedArgs(R"(^[a-zA-Z0-9 _\-\.]*$)");
    if (!std::regex_match(args, kAllowedArgs))
    {
        return json_response(400, {{"error", "Invalid characters in test arguments"}});
    }
    if (args.find("..") != std::string::npos)
    {
        return json_response(400, {{"error", "Path traversal not allowed in test arguments"}});
    }

    auto exe_dir = mo2core::executable_directory();
    auto py_path = exe_dir / "test.py";
    if (!fs::exists(py_path))
        return json_response(404,
                             {{"error", std::format("test.py not found in {}", exe_dir.string())}});

    // Build command line - test.py handles its own logging to test.log
    std::string cmd = std::format("python \"{}\" {}", py_path.string(), args);

    STARTUPINFOA si{};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi{};

    BOOL ok = CreateProcessA(nullptr,
                             cmd.data(),
                             nullptr,
                             nullptr,
                             FALSE,
                             CREATE_NO_WINDOW,
                             nullptr,
                             exe_dir.string().c_str(),
                             &si,
                             &pi);

    if (!ok)
    {
        auto err = GetLastError();
        return json_response(500,
                             {{"error", std::format("Failed to start test.py (error {})", err)}});
    }

    CloseHandle(pi.hThread);

    test_process_ = pi.hProcess;
    test_running_ = true;

    mo2core::Logger::instance().log(
        std::format("[server] Test suite started (PID {}) cmd: {}", pi.dwProcessId, cmd));
    return json_response(200, {{"running", true}, {"pid", static_cast<int>(pi.dwProcessId)}});
#else
    return json_response(501, {{"error", "Test runner only supported on Windows"}});
#endif
}

// ---------------------------------------------------------------------------
// GET /api/test/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_test_status()
{
#ifdef _WIN32
    std::lock_guard<std::mutex> lock(test_mutex_);

    if (!test_running_ || !test_process_)
    {
        return json_response(200, {{"running", false}});
    }

    DWORD wait_result = WaitForSingleObject(test_process_, 0);
    if (wait_result == WAIT_TIMEOUT)
    {
        return json_response(200, {{"running", true}});
    }
    if (wait_result == WAIT_FAILED)
    {
        mo2core::Logger::instance().log_warning(std::format(
            "[server] WaitForSingleObject failed in status check (error {})", GetLastError()));
        CloseHandle(test_process_);
        test_process_ = nullptr;
        test_running_ = false;
        return json_response(200,
                             {{"running", false}, {"error", "Failed to query process status"}});
    }

    // Process finished -- get actual exit code
    DWORD exit_code = 0;
    GetExitCodeProcess(test_process_, &exit_code);
    CloseHandle(test_process_);
    test_process_ = nullptr;
    test_running_ = false;

    return json_response(200, {{"running", false}, {"exitCode", static_cast<int>(exit_code)}});
#else
    return json_response(200, {{"running", false}});
#endif
}

}  // namespace mo2server

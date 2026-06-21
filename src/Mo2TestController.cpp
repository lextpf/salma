#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "Logger.hpp"
#include "Utils.hpp"

#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>
#include <regex>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

crow::response Mo2Controller::run_tests(const crow::request& req)
{
#ifdef _WIN32
    std::unique_lock<std::mutex> lock(test_mutex_);

    // reap a finished child that no status request observed.
    if (test_running_)
    {
        if (test_process_)
        {
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
            // release the handle after completion or a failed wait.
            CloseHandle(test_process_);
            test_process_ = nullptr;
            test_running_ = false;
        }
    }

    // invalid optional JSON falls back to an argument-free run.
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

    // allowlist interpolated arguments so they cannot add syntax or paths.
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
    auto py_path = exe_dir / "test_all.py";
    if (!fs::exists(py_path))
        return json_response(
            404, {{"error", std::format("test_all.py not found in {}", exe_dir.string())}});

    // test_all.py writes the log served by the test-log route.
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
        return json_response(
            500, {{"error", std::format("Failed to start test_all.py (error {})", err)}});
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

    // completion makes exit code 259 unambiguous.
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

#include "Mo2Controller.h"
#include "Mo2Helpers.h"

#include "ConfigService.h"
#include "Logger.h"
#include "Utils.h"

#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <format>
#include <memory>
#include <nlohmann/json.hpp>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// ---------------------------------------------------------------------------
// Static helpers
// ---------------------------------------------------------------------------

#ifdef _WIN32
// Format a Win32 error code into a human-readable message.
static std::string format_win32_error(DWORD code)
{
    char* buf = nullptr;
    DWORD len = FormatMessageA(
        FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        nullptr,
        code,
        0,
        reinterpret_cast<LPSTR>(&buf),
        0,
        nullptr);
    if (len == 0 || !buf)
        return std::format("error code {}", code);
    std::string msg(buf, len);
    LocalFree(buf);
    // Trim trailing whitespace/newline from FormatMessage output
    while (!msg.empty() && (msg.back() == '\n' || msg.back() == '\r' || msg.back() == ' '))
        msg.pop_back();
    return msg;
}

static int run_batch_script(const fs::path& script_path,
                            const fs::path& deploy_path,
                            const fs::path& mods_path)
{
    // Build an explicit environment block instead of mutating the process
    // environment with _putenv_s, which is not thread-safe.
    // RAII guard ensures the block is freed even if push_back throws.
    auto env_deleter = [](char* p)
    {
        if (p)
            FreeEnvironmentStringsA(p);
    };
    std::unique_ptr<char, decltype(env_deleter)> env_guard(GetEnvironmentStringsA(), env_deleter);
    std::vector<std::string> entries;
    if (env_guard)
    {
        for (const char* p = env_guard.get(); *p; p += std::strlen(p) + 1)
        {
            std::string entry(p);
            // Skip entries we are going to override
            if (entry.starts_with("SALMA_NO_PAUSE=") || entry.starts_with("SALMA_DEPLOY_PATH=") ||
                entry.starts_with("SALMA_MODS_PATH="))
            {
                continue;
            }
            entries.push_back(std::move(entry));
        }
    }
    env_guard.reset();
    entries.push_back("SALMA_NO_PAUSE=1");
    entries.push_back("SALMA_DEPLOY_PATH=" + deploy_path.string());
    entries.push_back("SALMA_MODS_PATH=" + mods_path.string());

    // Build the double-null-terminated environment block.
    std::vector<char> env_block;
    for (const auto& e : entries)
    {
        env_block.insert(env_block.end(), e.begin(), e.end());
        env_block.push_back('\0');
    }
    env_block.push_back('\0');

    // Use CreateProcessA instead of std::system to avoid shell injection.
    // The batch script path is from a known location (cwd), but deploy_path
    // and mods_path come from config/env and must not be passed through a shell.
    std::string cmd = std::format(R"(cmd.exe /c "call "{}"")", script_path.string());

    STARTUPINFOA si{};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi{};
    int exit_code = -1;

    if (CreateProcessA(nullptr,
                       cmd.data(),
                       nullptr,
                       nullptr,
                       FALSE,
                       CREATE_NO_WINDOW,
                       env_block.data(),
                       nullptr,
                       &si,
                       &pi))
    {
        WaitForSingleObject(pi.hProcess, INFINITE);
        DWORD code = 0;
        GetExitCodeProcess(pi.hProcess, &code);
        exit_code = static_cast<int>(code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }
    else
    {
        mo2core::Logger::instance().log_error(
            std::format("[server] CreateProcessA failed: {}", format_win32_error(GetLastError())));
    }

    return exit_code;
}
#endif

static bool path_contains_shell_metachar(const std::string& s)
{
    for (char c : s)
    {
        if (c == '&' || c == '|' || c == '>' || c == '<' || c == '^' || c == '%' || c == '!' ||
            c == '(' || c == ')' || c == '"')
        {
            return true;
        }
    }
    return false;
}

// ---------------------------------------------------------------------------
// Shared helper for deploy / purge plugin actions
// ---------------------------------------------------------------------------

crow::response Mo2Controller::run_plugin_action(const std::string& action)
{
    if (action != "deploy" && action != "purge")
    {
        return json_response(400, {{"error", "Invalid action"}});
    }

#ifdef _WIN32
    auto script_name = action + ".bat";
    auto script_path = fs::current_path() / script_name;
    if (!fs::exists(script_path))
    {
        return json_response(
            404,
            {{"error",
              std::format("{} not found in {}", script_name, fs::current_path().string())}});
    }

    auto& cfg = ConfigService::instance();
    fs::path mods_path = cfg.mo2_mods_path();
    if (mods_path.empty())
    {
        if (const char* mods_env = std::getenv("SALMA_MODS_PATH"); mods_env && *mods_env)
        {
            mods_path = fs::path(mods_env);
        }
    }
    if (mods_path.empty())
    {
        return json_response(
            400,
            {{"error", "MO2 mods path not configured. Set SALMA_MODS_PATH or configure via API"}});
    }
    fs::path deploy_path = resolve_deploy_path(mods_path.string());

    if (path_contains_shell_metachar(deploy_path.string()) ||
        path_contains_shell_metachar(mods_path.string()))
    {
        return json_response(400,
                             {{"error", "Paths contain invalid characters for batch execution"}});
    }

    bool started = plugin_action_job_.try_start(
        [action, script_name, script_path, deploy_path, mods_path]() -> PluginActionResult
        {
            auto& logger = mo2core::Logger::instance();
            logger.log(std::format("[server] Running {} script: {}", action, script_path.string()));
            int exit_code = run_batch_script(script_path, deploy_path, mods_path);
            logger.log(std::format("[server] {} script exit code: {}", action, exit_code));

            PluginActionResult r;
            r.success = (exit_code == 0);
            r.exit_code = exit_code;
            r.plugin_installed = plugin_installed_at(deploy_path);
            r.deploy_path = deploy_path.string();
            r.action = action;
            return r;
        });

    if (!started)
    {
        return json_response(409, {{"error", "Plugin action is already running"}});
    }

    return json_response(200, {{"started", true}, {"action", action}});
#else
    return json_response(501, {{"error", std::format("{} is only supported on Windows", action)}});
#endif
}

// ---------------------------------------------------------------------------
// POST /api/plugin/deploy
// ---------------------------------------------------------------------------

crow::response Mo2Controller::deploy_plugin()
{
    return run_plugin_action("deploy");
}

// ---------------------------------------------------------------------------
// POST /api/plugin/purge
// ---------------------------------------------------------------------------

crow::response Mo2Controller::purge_plugin()
{
    return run_plugin_action("purge");
}

// ---------------------------------------------------------------------------
// GET /api/plugin/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_plugin_action_status()
{
    json result = {{"running", plugin_action_job_.is_running()}};

    plugin_action_job_.read_result(
        [&](bool has_result, const PluginActionResult* r, const std::string& error)
        {
            if (!has_result || !r)
                return;
            result["success"] = r->success;
            result["exitCode"] = r->exit_code;
            result["pluginInstalled"] = r->plugin_installed;
            result["pluginDeployPath"] = r->deploy_path;
            result["action"] = r->action;
            if (!error.empty())
                result["error"] = error;
        });

    return json_response(200, result);
}

}  // namespace mo2server

// Mo2PluginController - deploy and purge the MO2 plugin by running the repo's
// own deploy.bat / purge.bat, and report what the last run did.
//
// This is the one place in the server that spawns a shell. The exposure is
// narrow by construction, and the order of the checks is the security policy:
//
//   run_plugin_action(action)
//     1. action must be exactly "deploy" or "purge"        else 400
//     2. script = <exe dir>/<action>.bat, must exist       else 404
//     3. mods path from ConfigService, else SALMA_MODS_PATH else 400
//     4. deploy path from resolve_deploy_path(mods path)
//     5. reject cmd.exe metacharacters in deploy and mods  else 400
//     6. one BackgroundJob at a time                       else 409
//     7. run_batch_script, then record exit code and
//        whether the plugin is now present at deploy path
//
// The child process gets an explicitly built environment block carrying
// SALMA_NO_PAUSE=1, SALMA_DEPLOY_PATH and SALMA_MODS_PATH. Building it by hand
// avoids _putenv_s, which mutates the whole process environment and is not
// thread-safe; this server is multithreaded. SALMA_NO_PAUSE=1 is required:
// without it the script's trailing `pause` blocks forever on a console the
// server cannot answer.
//
// run_batch_script blocks the background worker for up to 30 minutes, then
// terminates the child. Exit codes it returns: the script's own code, -1 when
// CreateProcessA failed, -2 on the timeout.

#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "ConfigService.hpp"
#include "Logger.hpp"
#include "Utils.hpp"

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
// Format a Win32 error code as text, falling back to "error code N" when
// FormatMessageA has nothing for it.
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
    // FormatMessageA appends a trailing newline; strip it so the text embeds
    // cleanly in a log line.
    while (!msg.empty() && (msg.back() == '\n' || msg.back() == '\r' || msg.back() == ' '))
        msg.pop_back();
    return msg;
}

// Run one batch script to completion and return its exit code, or -1 when the
// process could not be started and -2 when it was terminated on the 30-minute
// timeout. Blocks the calling thread for the whole run, so call it only from a
// BackgroundJob worker.
static int run_batch_script(const fs::path& script_path,
                            const fs::path& deploy_path,
                            const fs::path& mods_path)
{
    // Copy the current environment, minus the three variables overridden below.
    // The guard frees the OS block even if a push_back throws.
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

    // CreateProcessA wants the block double-null-terminated.
    std::vector<char> env_block;
    for (const auto& e : entries)
    {
        env_block.insert(env_block.end(), e.begin(), e.end());
        env_block.push_back('\0');
    }
    env_block.push_back('\0');

    // `script_path` is the only value that reaches the cmd.exe command line.
    // deploy_path and mods_path travel in the environment block built above,
    // where shell metacharacters mean nothing. run_plugin_action still screens
    // those two with path_contains_shell_metachar, to cover whatever the batch
    // script itself does with them.
    //
    // script_path goes unscreened, on a precondition: it is <exe dir>/deploy.bat
    // or <exe dir>/purge.bat, built from mo2core::executable_directory() plus a
    // literal filename, and the installation directory is assumed free of
    // cmd.exe metacharacters. If that assumption stops holding, screen
    // script_path here rather than trusting the quoting below.
    //
    // A shell does parse this: the image is cmd.exe and /c hands it the rest.
    // CreateProcessA only removes the second, implicit shell that std::system
    // would add. The quoting below is what stands between script_path and that
    // shell, which is why the precondition above has to keep holding.
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
        // A hung script would otherwise hold the background worker forever, and
        // no further plugin action could start.
        constexpr DWORD kScriptTimeoutMs = 30 * 60 * 1000;
        DWORD wait = WaitForSingleObject(pi.hProcess, kScriptTimeoutMs);
        if (wait == WAIT_TIMEOUT)
        {
            mo2core::Logger::instance().log_error(
                "[server] Batch script timed out after 30 minutes, terminating");
            TerminateProcess(pi.hProcess, 1);
            WaitForSingleObject(pi.hProcess, 5000);
            exit_code = -2;
        }
        else
        {
            DWORD code = 0;
            GetExitCodeProcess(pi.hProcess, &code);
            exit_code = static_cast<int>(code);
        }
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

// Reject the cmd.exe metacharacters a path could carry. A denylist rather than
// an allowlist because the inputs are real user directory paths, and an
// allowlist would reject legitimate names. run_batch_script says which values
// are screened and why script_path is not among them.
//
// The list is not the complete cmd.exe special set, and does not need to be:
// these values reach the child through the environment block, never the command
// line. Comma, equals and tab are cmd token delimiters and are absent here;
// single quote and backtick are present although cmd treats neither specially.
// Widen it if a screened value ever starts reaching a command line.
static bool path_contains_shell_metachar(const std::string& s)
{
    for (char c : s)
    {
        if (c == '&' || c == '|' || c == '>' || c == '<' || c == '^' || c == '%' || c == '!' ||
            c == '(' || c == ')' || c == '"' || c == ';' || c == '\'' || c == '`')
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
    auto script_dir = mo2core::executable_directory();
    auto script_path = script_dir / script_name;
    if (!fs::exists(script_path))
    {
        return json_response(
            404, {{"error", std::format("{} not found in {}", script_name, script_dir.string())}});
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
    // `running` is read outside read_result's mutex-held callback, so a poll can
    // briefly see running=true next to a completed result. Acceptable for the
    // same reason as get_scan_status: the dashboard polls on a timer and the
    // next poll corrects it. InstallationController::handle_status shows the
    // consistent pattern and why that one needs it.
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

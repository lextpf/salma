#include "Mo2Controller.h"
#include "ConfigService.h"
#include "FomodInferenceService.h"
#include "Logger.h"
#include "Utils.h"

#include <algorithm>
#include <cctype>
#include <charconv>
#include <chrono>
#include <cstdlib>
#include <deque>
#include <filesystem>
#include <format>
#include <fstream>
#include <nlohmann/json.hpp>
#include <thread>
#include <vector>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// ---------------------------------------------------------------------------
// Constructor / Destructor
// ---------------------------------------------------------------------------

Mo2Controller::Mo2Controller() = default;

Mo2Controller::~Mo2Controller()
{
    // Join background threads to avoid use-after-free on shutdown
    if (scan_thread_.joinable())
    {
        scan_thread_.join();
    }
    if (plugin_action_thread_.joinable())
    {
        plugin_action_thread_.join();
    }
#ifdef _WIN32
    if (test_process_)
    {
        CloseHandle(test_process_);
        test_process_ = nullptr;
    }
#endif
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static crow::response json_response(int code, const json& j)
{
    crow::response res(code, j.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

/// Percent-decode a URL-encoded string (%XX sequences).
static std::string url_decode(const std::string& src)
{
    std::string out;
    out.reserve(src.size());
    for (size_t i = 0; i < src.size(); ++i)
    {
        if (src[i] == '%' && i + 2 < src.size())
        {
            unsigned int ch = 0;
            auto [ptr, ec] = std::from_chars(src.data() + i + 1, src.data() + i + 3, ch, 16);
            if (ec == std::errc{} && ptr == src.data() + i + 3)
            {
                out += static_cast<char>(ch);
                i += 2;
                continue;
            }
        }
        out += (src[i] == '+') ? ' ' : src[i];
    }
    return out;
}

/// Validate that `child` is strictly inside `parent` (no traversal).
static bool is_inside(const fs::path& parent, const fs::path& child)
{
    auto rel = fs::weakly_canonical(child).lexically_relative(fs::weakly_canonical(parent));
    return !rel.empty() && !rel.string().starts_with("..");
}

static std::string trim_copy(const std::string& s)
{
    size_t start = 0;
    while (start < s.size() && std::isspace(static_cast<unsigned char>(s[start])))
    {
        ++start;
    }
    size_t end = s.size();
    while (end > start && std::isspace(static_cast<unsigned char>(s[end - 1])))
    {
        --end;
    }
    return s.substr(start, end - start);
}

static std::string to_lower_copy(const std::string& s)
{
    return mo2core::to_lower(s);
}

// Parse [General] installationFile from MO2 meta.ini.
static std::string read_installation_file(const fs::path& meta_ini_path)
{
    std::ifstream ifs(meta_ini_path);
    if (!ifs)
    {
        return "";
    }

    bool in_general = false;
    std::string line;
    while (std::getline(ifs, line))
    {
        if (!line.empty() && line.back() == '\r')
        {
            line.pop_back();
        }

        auto trimmed = trim_copy(line);
        if (trimmed.empty() || trimmed[0] == ';' || trimmed[0] == '#')
        {
            continue;
        }

        if (trimmed.front() == '[' && trimmed.back() == ']')
        {
            auto section = to_lower_copy(trim_copy(trimmed.substr(1, trimmed.size() - 2)));
            in_general = (section == "general");
            continue;
        }

        if (!in_general)
        {
            continue;
        }

        auto eq = trimmed.find('=');
        if (eq == std::string::npos)
        {
            continue;
        }

        auto key = to_lower_copy(trim_copy(trimmed.substr(0, eq)));
        if (key != "installationfile")
        {
            continue;
        }

        auto value = trim_copy(trimmed.substr(eq + 1));
        if (value.size() >= 2)
        {
            const char first = value.front();
            const char last = value.back();
            if ((first == '"' && last == '"') || (first == '\'' && last == '\''))
            {
                value = value.substr(1, value.size() - 2);
            }
        }
        return value;
    }

    return "";
}

static fs::path resolve_archive_path(const std::string& archive_value,
                                     const fs::path& mod_folder,
                                     const fs::path& mods_dir)
{
    if (archive_value.empty())
    {
        return {};
    }

    fs::path archive_path = fs::path(archive_value);
    if (archive_path.is_absolute())
    {
        return fs::exists(archive_path) ? archive_path : fs::path{};
    }

    std::vector<fs::path> candidates;

    if (const char* downloads = std::getenv("SALMA_DOWNLOADS_PATH"); downloads && *downloads)
    {
        candidates.push_back(fs::path(downloads) / archive_path);
    }

    // Extra fallbacks for common local setups.
    candidates.push_back(mod_folder / archive_path);
    if (!mods_dir.empty())
    {
        auto parent = mods_dir.parent_path();
        candidates.push_back(parent / archive_path);
        candidates.push_back(parent / "downloads" / archive_path);
        if (parent.has_parent_path())
        {
            candidates.push_back(parent.parent_path() / "downloads" / archive_path);
        }
    }

    for (const auto& candidate : candidates)
    {
        if (fs::exists(candidate))
        {
            return candidate;
        }
    }

    return {};
}

static fs::path resolve_deploy_path(const std::string& mo2_mods_path)
{
    if (const char* deploy_env = std::getenv("SALMA_DEPLOY_PATH"); deploy_env && *deploy_env)
    {
        return fs::path(deploy_env);
    }

    fs::path mods_path = mo2_mods_path;
    if (mods_path.empty())
    {
        if (const char* mods_env = std::getenv("SALMA_MODS_PATH"); mods_env && *mods_env)
        {
            mods_path = fs::path(mods_env);
        }
    }

    if (!mods_path.empty())
    {
        auto instance_root = mods_path.parent_path().parent_path();
        if (!instance_root.empty())
        {
            return instance_root / "MO2" / "plugins";
        }
    }

    mo2core::Logger::instance().log_error(
        "[server] Cannot determine plugin deploy path: "
        "set SALMA_DEPLOY_PATH or configure MO2 mods path");
    return {};
}

static bool plugin_installed_at(const fs::path& deploy_path)
{
    if (deploy_path.empty())
    {
        return false;
    }
    return fs::exists(deploy_path / "salma" / "mo2-salma.dll") &&
           fs::exists(deploy_path / "mo2-salma.py");
}

#ifdef _WIN32
static int run_batch_script(const fs::path& script_path,
                            const fs::path& deploy_path,
                            const fs::path& mods_path)
{
    struct EnvBackup
    {
        std::string name;
        std::string value;
        bool had_value;
    };

    std::vector<EnvBackup> backups;
    auto set_env = [&](const char* name, const std::string& value)
    {
        const char* old = std::getenv(name);
        backups.push_back({name, old ? old : "", old != nullptr});
        _putenv_s(name, value.c_str());
    };

    set_env("SALMA_NO_PAUSE", "1");
    set_env("SALMA_DEPLOY_PATH", deploy_path.string());
    set_env("SALMA_MODS_PATH", mods_path.string());

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
                       nullptr,
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

    for (const auto& backup : backups)
    {
        _putenv_s(backup.name.c_str(), backup.had_value ? backup.value.c_str() : "");
    }

    return exit_code;
}
#endif

static json run_fomod_scan_job(const fs::path& mods_dir, const fs::path& output_dir)
{
    auto& logger = mo2core::Logger::instance();

    std::vector<fs::path> mod_folders;
    for (const auto& entry : fs::directory_iterator(mods_dir))
    {
        if (!entry.is_directory())
        {
            continue;
        }
        if (entry.path().filename() == "Salma FOMODs Output")
        {
            continue;
        }
        mod_folders.push_back(entry.path());
    }

    std::sort(mod_folders.begin(),
              mod_folders.end(),
              [](const fs::path& a, const fs::path& b)
              { return a.filename().string() < b.filename().string(); });

    int total = static_cast<int>(mod_folders.size());
    int scanned = 0;
    int inferred = 0;
    int no_fomod = 0;
    int skipped_existing = 0;
    int no_archive = 0;
    int archive_missing = 0;
    int errors = 0;

    auto started = std::chrono::steady_clock::now();
    logger.log(std::format("[infer] Starting scan in: {}", mods_dir.string()));
    logger.log(std::format("[infer] Output dir: {}", output_dir.string()));
    logger.log(std::format("[infer] Found {} mod folders", total));

    constexpr size_t kInferDotColumn = 112;
    auto infer_status = [total](int index,
                                const std::string& mod_name,
                                const std::string& status,
                                const std::string& detail) -> std::string
    {
        std::string label = std::format("[infer] [{}/{}] {}", index + 1, total, mod_name);
        size_t dots_count = 4;
        if (label.size() < kInferDotColumn)
        {
            dots_count = kInferDotColumn - label.size();
        }
        std::string dots(dots_count, '.');
        if (detail.empty())
        {
            return std::format("{} {} {}", label, dots, status);
        }
        return std::format("{} {} {} {}", label, dots, status, detail);
    };

    for (int i = 0; i < total; ++i)
    {
        const fs::path& mod_folder = mod_folders[i];
        const std::string mod_name = mod_folder.filename().string();
        const fs::path choices_file = output_dir / (mod_name + ".json");

        if (fs::exists(choices_file))
        {
            ++skipped_existing;
            logger.log(infer_status(i, mod_name, "SKIP", "(exists)"));
            continue;
        }

        const std::string archive_value = read_installation_file(mod_folder / "meta.ini");
        if (archive_value.empty())
        {
            ++no_archive;
            logger.log(infer_status(i, mod_name, "SKIP", "(no archive)"));
            continue;
        }

        fs::path archive_path = resolve_archive_path(archive_value, mod_folder, mods_dir);
        if (archive_path.empty() || !fs::exists(archive_path))
        {
            ++archive_missing;
            logger.log(infer_status(i, mod_name, "SKIP", "(archive missing)"));
            continue;
        }

        ++scanned;
        logger.log(
            std::format(R"([infer] [{}/{}]   Archive: "{}")", i + 1, total, archive_path.string()));

        try
        {
            auto item_start = std::chrono::steady_clock::now();
            mo2core::FomodInferenceService service;
            std::string result =
                service.infer_selections(archive_path.string(), mod_folder.string());
            double elapsed_s =
                std::chrono::duration<double>(std::chrono::steady_clock::now() - item_start)
                    .count();

            if (result.empty())
            {
                ++no_fomod;
                logger.log(infer_status(
                    i, mod_name, "NOT FOMOD", std::format("({:.1f}s): no FOMOD data", elapsed_s)));
                continue;
            }

            auto parsed = json::parse(result, nullptr, false);
            if (parsed.is_discarded())
            {
                ++errors;
                logger.log_error(infer_status(i, mod_name, "ERROR", "(invalid JSON returned)"));
                continue;
            }

            if (!parsed.contains("steps") || !parsed["steps"].is_array() || parsed["steps"].empty())
            {
                ++no_fomod;
                logger.log(infer_status(
                    i, mod_name, "NO STEPS", std::format("({:.1f}s): no FOMOD steps", elapsed_s)));
                continue;
            }

            std::ofstream ofs(choices_file);
            if (!ofs)
            {
                ++errors;
                logger.log_error(
                    infer_status(i,
                                 mod_name,
                                 "ERROR",
                                 std::format("(failed to write {})", choices_file.string())));
                continue;
            }

            ofs << parsed.dump(2);
            ++inferred;
            logger.log(infer_status(
                i,
                mod_name,
                "INFERRED",
                std::format("({:.1f}s): {} steps saved", elapsed_s, parsed["steps"].size())));
        }
        catch (const std::exception& ex)
        {
            ++errors;
            logger.log_error(infer_status(i, mod_name, "ERROR", std::format("({})", ex.what())));
        }
    }

    auto duration_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                           std::chrono::steady_clock::now() - started)
                           .count();

    json summary = {{"success", true},
                    {"totalModFolders", total},
                    {"archivesProcessed", scanned},
                    {"choicesInferred", inferred},
                    {"noFomod", no_fomod},
                    {"alreadyHadChoices", skipped_existing},
                    {"noArchiveFound", no_archive},
                    {"archiveMissing", archive_missing},
                    {"errors", errors},
                    {"durationMs", duration_ms},
                    {"outputDir", output_dir.string()}};

    logger.log(
        std::format("[infer] Scan complete: total={} processed={} inferred={} no_fomod={} "
                    "existing={} no_archive={} archive_missing={} errors={} duration={}ms",
                    total,
                    scanned,
                    inferred,
                    no_fomod,
                    skipped_existing,
                    no_archive,
                    archive_missing,
                    errors,
                    duration_ms));

    return summary;
}

// ---------------------------------------------------------------------------
// GET /api/config
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_config()
{
    auto& cfg = ConfigService::instance();
    auto mods = cfg.mo2_mods_path();
    auto fomod_dir = cfg.fomod_output_dir();

    json j = {{"mo2ModsPath", mods},
              {"fomodOutputDir", fomod_dir.string()},
              {"mo2ModsPathValid", !mods.empty() && fs::is_directory(mods)}};
    return json_response(200, j);
}

// ---------------------------------------------------------------------------
// PUT /api/config
// ---------------------------------------------------------------------------

crow::response Mo2Controller::put_config(const crow::request& req)
{
    try
    {
        auto body = json::parse(req.body);
        auto& cfg = ConfigService::instance();

        if (body.contains("mo2ModsPath"))
            cfg.set_mo2_mods_path(body["mo2ModsPath"].get<std::string>());

        cfg.save();
        return get_config();
    }
    catch (const std::exception& ex)
    {
        return json_response(400, {{"error", ex.what()}});
    }
}

// ---------------------------------------------------------------------------
// GET /api/mo2/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_status()
{
    auto& cfg = ConfigService::instance();
    auto fomod_dir = cfg.fomod_output_dir();
    auto mods_path = cfg.mo2_mods_path();
    auto deploy_path = resolve_deploy_path(mods_path);
    bool plugin_installed = plugin_installed_at(deploy_path);

    bool output_exists = !fomod_dir.empty() && fs::is_directory(fomod_dir);
    int json_count = 0;
    int mod_count = 0;

    if (output_exists)
    {
        for (auto& entry : fs::directory_iterator(fomod_dir))
        {
            if (entry.is_regular_file() && entry.path().extension() == ".json")
                ++json_count;
        }
    }

    // Count mod folders under the mods path (top-level directories)
    if (!mods_path.empty() && fs::is_directory(mods_path))
    {
        for (auto& entry : fs::directory_iterator(mods_path))
        {
            if (entry.is_directory())
                ++mod_count;
        }
    }

    json j = {{"configured", !cfg.mo2_mods_path().empty()},
              {"outputFolderExists", output_exists},
              {"fomodOutputDir", fomod_dir.string()},
              {"jsonCount", json_count},
              {"modCount", mod_count},
              {"pluginInstalled", plugin_installed},
              {"pluginDeployPath", deploy_path.string()}};
    return json_response(200, j);
}

// ---------------------------------------------------------------------------
// GET /api/mo2/fomods
// ---------------------------------------------------------------------------

crow::response Mo2Controller::list_fomods()
{
    auto fomod_dir = ConfigService::instance().fomod_output_dir();
    if (fomod_dir.empty() || !fs::is_directory(fomod_dir))
    {
        return json_response(200, json::array());
    }

    json list = json::array();
    for (auto& entry : fs::directory_iterator(fomod_dir))
    {
        if (!entry.is_regular_file() || entry.path().extension() != ".json")
            continue;

        auto name = entry.path().stem().string();
        auto size = entry.file_size();
        auto ftime = fs::last_write_time(entry);
        auto sctp = std::chrono::clock_cast<std::chrono::system_clock>(ftime);
        auto epoch =
            std::chrono::duration_cast<std::chrono::milliseconds>(sctp.time_since_epoch()).count();

        // Count steps by reading the JSON
        int step_count = 0;
        try
        {
            std::ifstream ifs(entry.path());
            auto j = json::parse(ifs);
            if (j.contains("steps") && j["steps"].is_array())
                step_count = static_cast<int>(j["steps"].size());
        }
        catch (const std::exception& ex)
        {
            mo2core::Logger::instance().log_warning(std::format(
                "[server] Failed to read steps from {}: {}", entry.path().string(), ex.what()));
        }
        catch (...)
        {
        }

        list.push_back(
            {{"name", name}, {"size", size}, {"modified", epoch}, {"stepCount", step_count}});
    }
    return json_response(200, list);
}

// ---------------------------------------------------------------------------
// POST /api/mo2/fomods/scan
// ---------------------------------------------------------------------------

crow::response Mo2Controller::scan_fomods()
{
    auto& cfg = ConfigService::instance();

    const fs::path mods_dir = cfg.mo2_mods_path();
    const fs::path output_dir = cfg.fomod_output_dir();

    if (mods_dir.empty() || !fs::is_directory(mods_dir))
    {
        return json_response(400, {{"error", "MO2 mods path is not configured or does not exist"}});
    }
    if (output_dir.empty())
    {
        return json_response(400, {{"error", "FOMOD output directory is not configured"}});
    }

    try
    {
        fs::create_directories(output_dir);
    }
    catch (const std::exception& ex)
    {
        return json_response(
            500, {{"error", std::format("Failed to create output directory: {}", ex.what())}});
    }

    if (scan_running_.exchange(true))
    {
        return json_response(409, {{"error", "FOMOD scan is already running"}});
    }

    {
        std::lock_guard<std::mutex> lock(scan_mutex_);
        scan_has_result_ = false;
        scan_last_success_ = false;
        scan_last_error_.clear();
        scan_total_mod_folders_ = 0;
        scan_archives_processed_ = 0;
        scan_choices_inferred_ = 0;
        scan_no_fomod_ = 0;
        scan_already_had_choices_ = 0;
        scan_no_archive_found_ = 0;
        scan_archive_missing_ = 0;
        scan_errors_ = 0;
        scan_duration_ms_ = 0;
        scan_output_dir_ = output_dir.string();
    }

    if (scan_thread_.joinable())
    {
        scan_thread_.join();
    }
    scan_thread_ = std::thread(
        [this, mods_dir, output_dir]()
        {
#ifdef _WIN32
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
#endif
            try
            {
                json summary = run_fomod_scan_job(mods_dir, output_dir);
                std::lock_guard<std::mutex> lock(scan_mutex_);
                scan_has_result_ = true;
                scan_last_success_ = summary.value("success", false);
                scan_total_mod_folders_ = summary.value("totalModFolders", 0);
                scan_archives_processed_ = summary.value("archivesProcessed", 0);
                scan_choices_inferred_ = summary.value("choicesInferred", 0);
                scan_no_fomod_ = summary.value("noFomod", 0);
                scan_already_had_choices_ = summary.value("alreadyHadChoices", 0);
                scan_no_archive_found_ = summary.value("noArchiveFound", 0);
                scan_archive_missing_ = summary.value("archiveMissing", 0);
                scan_errors_ = summary.value("errors", 0);
                scan_duration_ms_ = summary.value("durationMs", 0LL);
                scan_output_dir_ = summary.value("outputDir", std::string{});
                scan_last_error_.clear();
            }
            catch (const std::exception& ex)
            {
                mo2core::Logger::instance().log_error(
                    std::format("[infer] Scan failed: {}", ex.what()));
                std::lock_guard<std::mutex> lock(scan_mutex_);
                scan_has_result_ = true;
                scan_last_success_ = false;
                scan_last_error_ = ex.what();
            }
            catch (...)
            {
                mo2core::Logger::instance().log_error("[infer] Scan failed: unknown error");
                std::lock_guard<std::mutex> lock(scan_mutex_);
                scan_has_result_ = true;
                scan_last_success_ = false;
                scan_last_error_ = "Unknown scan error";
            }
            scan_running_.store(false);
        });

    return json_response(200, {{"success", true}, {"running", true}, {"started", true}});
}

// ---------------------------------------------------------------------------
// GET /api/mo2/fomods/scan/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_scan_status()
{
    json result = {{"running", scan_running_.load()}};

    std::lock_guard<std::mutex> lock(scan_mutex_);
    if (scan_has_result_)
    {
        result["success"] = scan_last_success_;
        result["totalModFolders"] = scan_total_mod_folders_;
        result["archivesProcessed"] = scan_archives_processed_;
        result["choicesInferred"] = scan_choices_inferred_;
        result["noFomod"] = scan_no_fomod_;
        result["alreadyHadChoices"] = scan_already_had_choices_;
        result["noArchiveFound"] = scan_no_archive_found_;
        result["archiveMissing"] = scan_archive_missing_;
        result["errors"] = scan_errors_;
        result["durationMs"] = scan_duration_ms_;
        result["outputDir"] = scan_output_dir_;
        if (!scan_last_error_.empty())
        {
            result["error"] = scan_last_error_;
        }
    }

    return json_response(200, result);
}

// ---------------------------------------------------------------------------
// GET /api/mo2/fomods/<name>
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_fomod(const std::string& name)
{
    auto decoded = url_decode(name);
    auto fomod_dir = ConfigService::instance().fomod_output_dir();
    if (fomod_dir.empty())
        return json_response(404, {{"error", "FOMOD output directory not configured"}});

    auto file_path = fomod_dir / (decoded + ".json");

    auto& logger = mo2core::Logger::instance();
    logger.log(
        std::format("[server] get_fomod: name=\"{}\" path=\"{}\"", decoded, file_path.string()));

    if (!is_inside(fomod_dir, file_path))
        return json_response(403, {{"error", "Path traversal rejected"}});

    if (!fs::exists(file_path))
        return json_response(404, {{"error", "FOMOD JSON not found"}});

    try
    {
        std::ifstream ifs(file_path);
        auto content = json::parse(ifs);
        return json_response(200, content);
    }
    catch (const std::exception& ex)
    {
        return json_response(500, {{"error", std::format("Failed to read: {}", ex.what())}});
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/mo2/fomods/<name>
// ---------------------------------------------------------------------------

crow::response Mo2Controller::delete_fomod(const std::string& name)
{
    auto decoded = url_decode(name);
    auto fomod_dir = ConfigService::instance().fomod_output_dir();
    if (fomod_dir.empty())
        return json_response(404, {{"error", "FOMOD output directory not configured"}});

    auto file_path = fomod_dir / (decoded + ".json");

    if (!is_inside(fomod_dir, file_path))
        return json_response(403, {{"error", "Path traversal rejected"}});

    if (!fs::exists(file_path))
        return json_response(404, {{"error", "FOMOD JSON not found"}});

    try
    {
        fs::remove(file_path);
        mo2core::Logger::instance().log(std::format("[server] Deleted FOMOD JSON: {}", decoded));
        return json_response(200, {{"success", true}});
    }
    catch (const std::exception& ex)
    {
        return json_response(500, {{"error", std::format("Failed to delete: {}", ex.what())}});
    }
}

// ---------------------------------------------------------------------------
// POST /api/plugin/deploy
// ---------------------------------------------------------------------------

crow::response Mo2Controller::deploy_plugin()
{
#ifdef _WIN32
    if (plugin_action_running_.exchange(true))
    {
        return json_response(409, {{"error", "Plugin action is already running"}});
    }

    auto script_path = fs::current_path() / "deploy.bat";
    if (!fs::exists(script_path))
    {
        plugin_action_running_.store(false);
        return json_response(
            404,
            {{"error", std::format("deploy.bat not found in {}", fs::current_path().string())}});
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
        plugin_action_running_.store(false);
        return json_response(
            400,
            {{"error", "MO2 mods path not configured. Set SALMA_MODS_PATH or configure via API"}});
    }
    fs::path deploy_path = resolve_deploy_path(mods_path.string());

    {
        std::lock_guard<std::mutex> lock(plugin_action_mutex_);
        plugin_action_has_result_ = false;
        plugin_action_type_ = "deploy";
        plugin_action_last_error_.clear();
    }

    if (plugin_action_thread_.joinable())
    {
        plugin_action_thread_.join();
    }
    plugin_action_thread_ = std::thread(
        [this, script_path, deploy_path, mods_path]()
        {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
            auto& logger = mo2core::Logger::instance();
            try
            {
                logger.log(std::format("[server] Running deploy script: {}", script_path.string()));
                int exit_code = run_batch_script(script_path, deploy_path, mods_path);
                logger.log(std::format("[server] Deploy script exit code: {}", exit_code));

                std::lock_guard<std::mutex> lock(plugin_action_mutex_);
                plugin_action_has_result_ = true;
                plugin_action_last_success_ = (exit_code == 0);
                plugin_action_exit_code_ = exit_code;
                plugin_action_plugin_installed_ = plugin_installed_at(deploy_path);
                plugin_action_deploy_path_ = deploy_path.string();
                if (exit_code != 0)
                {
                    plugin_action_last_error_ =
                        std::format("deploy.bat failed with exit code {}", exit_code);
                }
            }
            catch (const std::exception& ex)
            {
                logger.log_error(std::format("[server] Deploy failed: {}", ex.what()));
                std::lock_guard<std::mutex> lock(plugin_action_mutex_);
                plugin_action_has_result_ = true;
                plugin_action_last_success_ = false;
                plugin_action_last_error_ = ex.what();
            }
            plugin_action_running_.store(false);
        });

    return json_response(200, {{"started", true}, {"action", "deploy"}});
#else
    return json_response(501, {{"error", "Deploy is only supported on Windows"}});
#endif
}

// ---------------------------------------------------------------------------
// POST /api/plugin/purge
// ---------------------------------------------------------------------------

crow::response Mo2Controller::purge_plugin()
{
#ifdef _WIN32
    if (plugin_action_running_.exchange(true))
    {
        return json_response(409, {{"error", "Plugin action is already running"}});
    }

    auto script_path = fs::current_path() / "purge.bat";
    if (!fs::exists(script_path))
    {
        plugin_action_running_.store(false);
        return json_response(
            404,
            {{"error", std::format("purge.bat not found in {}", fs::current_path().string())}});
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
        plugin_action_running_.store(false);
        return json_response(
            400,
            {{"error", "MO2 mods path not configured. Set SALMA_MODS_PATH or configure via API"}});
    }
    fs::path deploy_path = resolve_deploy_path(mods_path.string());

    {
        std::lock_guard<std::mutex> lock(plugin_action_mutex_);
        plugin_action_has_result_ = false;
        plugin_action_type_ = "purge";
        plugin_action_last_error_.clear();
    }

    if (plugin_action_thread_.joinable())
    {
        plugin_action_thread_.join();
    }
    plugin_action_thread_ = std::thread(
        [this, script_path, deploy_path, mods_path]()
        {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
            auto& logger = mo2core::Logger::instance();
            try
            {
                logger.log(std::format("[server] Running purge script: {}", script_path.string()));
                int exit_code = run_batch_script(script_path, deploy_path, mods_path);
                logger.log(std::format("[server] Purge script exit code: {}", exit_code));

                std::lock_guard<std::mutex> lock(plugin_action_mutex_);
                plugin_action_has_result_ = true;
                plugin_action_last_success_ = (exit_code == 0);
                plugin_action_exit_code_ = exit_code;
                plugin_action_plugin_installed_ = plugin_installed_at(deploy_path);
                plugin_action_deploy_path_ = deploy_path.string();
                if (exit_code != 0)
                {
                    plugin_action_last_error_ =
                        std::format("purge.bat failed with exit code {}", exit_code);
                }
            }
            catch (const std::exception& ex)
            {
                logger.log_error(std::format("[server] Purge failed: {}", ex.what()));
                std::lock_guard<std::mutex> lock(plugin_action_mutex_);
                plugin_action_has_result_ = true;
                plugin_action_last_success_ = false;
                plugin_action_last_error_ = ex.what();
            }
            plugin_action_running_.store(false);
        });

    return json_response(200, {{"started", true}, {"action", "purge"}});
#else
    return json_response(501, {{"error", "Purge is only supported on Windows"}});
#endif
}

// ---------------------------------------------------------------------------
// GET /api/plugin/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_plugin_action_status()
{
    json result = {{"running", plugin_action_running_.load()}};
    std::lock_guard<std::mutex> lock(plugin_action_mutex_);
    if (plugin_action_has_result_)
    {
        result["success"] = plugin_action_last_success_;
        result["exitCode"] = plugin_action_exit_code_;
        result["pluginInstalled"] = plugin_action_plugin_installed_;
        result["pluginDeployPath"] = plugin_action_deploy_path_;
        result["action"] = plugin_action_type_;
        if (!plugin_action_last_error_.empty())
            result["error"] = plugin_action_last_error_;
    }
    return json_response(200, result);
}

// ---------------------------------------------------------------------------
// Shared log-reading helper (used by get_logs and get_test_logs)
// ---------------------------------------------------------------------------

static crow::response read_log_file(const fs::path& log_path, const crow::request& req)
{
    static constexpr int kMaxLinesLimit = 5000;
    int max_lines = 100;
    auto lines_param = req.url_params.get("lines");
    if (lines_param)
    {
        try
        {
            max_lines = std::stoi(lines_param);
        }
        catch (...)
        {
        }
        if (max_lines < 0)
        {
            max_lines = 0;
        }
        if (max_lines > kMaxLinesLimit)
        {
            max_lines = kMaxLinesLimit;
        }
    }

    int64_t offset = -1;
    auto offset_param = req.url_params.get("offset");
    if (offset_param)
    {
        try
        {
            offset = std::stoll(offset_param);
        }
        catch (...)
        {
        }
        if (offset < 0)
            offset = -1;
    }

    if (!fs::exists(log_path))
        return json_response(200,
                             {{"lines", json::array()},
                              {"errors", 0},
                              {"warnings", 0},
                              {"passes", 0},
                              {"nextOffset", 0}});

    auto file_size = static_cast<int64_t>(fs::file_size(log_path));

    // Incremental mode: seek to offset, read only new lines
    if (offset >= 0)
    {
        // File was truncated (log cleared) - tell client to reset
        if (offset > file_size)
        {
            return json_response(200,
                                 {{"lines", json::array()},
                                  {"errors", 0},
                                  {"warnings", 0},
                                  {"passes", 0},
                                  {"nextOffset", 0},
                                  {"reset", true}});
        }

        // No new data
        if (offset == file_size)
        {
            return json_response(200,
                                 {{"lines", json::array()},
                                  {"errors", 0},
                                  {"warnings", 0},
                                  {"passes", 0},
                                  {"nextOffset", file_size}});
        }

        std::ifstream ifs(log_path, std::ios::binary);
        ifs.seekg(offset);
        json lines_arr = json::array();
        std::string line;
        int errors = 0, warnings = 0, passes = 0;
        while (std::getline(ifs, line))
        {
            // Strip trailing \r from Windows line endings
            if (!line.empty() && line.back() == '\r')
                line.pop_back();
            if (line.find("ERROR") != std::string::npos ||
                line.find("CRITICAL") != std::string::npos ||
                line.find("FATAL") != std::string::npos || line.find("FAIL") != std::string::npos)
                errors++;
            else if (line.find("WARNING") != std::string::npos ||
                     line.find("WARN") != std::string::npos)
                warnings++;
            else if (line.find("PASS") != std::string::npos ||
                     line.find("INFERRED") != std::string::npos)
                passes++;
            lines_arr.push_back(std::move(line));
        }

        return json_response(200,
                             {{"lines", lines_arr},
                              {"errors", errors},
                              {"warnings", warnings},
                              {"passes", passes},
                              {"nextOffset", file_size}});
    }

    // Full mode: read last N lines, count stats across ALL lines
    std::ifstream ifs(log_path);
    std::deque<std::string> tail;
    std::string line;
    int errors = 0, warnings = 0, passes = 0;
    while (std::getline(ifs, line))
    {
        if (line.find("ERROR") != std::string::npos || line.find("CRITICAL") != std::string::npos ||
            line.find("FATAL") != std::string::npos || line.find("FAIL") != std::string::npos)
            errors++;
        else if (line.find("WARNING") != std::string::npos ||
                 line.find("WARN") != std::string::npos)
            warnings++;
        else if (line.find("PASS") != std::string::npos ||
                 line.find("INFERRED") != std::string::npos)
            passes++;
        tail.push_back(std::move(line));
        if (max_lines > 0 && static_cast<int>(tail.size()) > max_lines)
            tail.pop_front();
    }

    json lines_arr = json::array();
    for (auto& l : tail)
        lines_arr.push_back(l);

    return json_response(200,
                         {{"lines", lines_arr},
                          {"errors", errors},
                          {"warnings", warnings},
                          {"passes", passes},
                          {"nextOffset", file_size}});
}

// ---------------------------------------------------------------------------
// GET /api/logs?lines=N&offset=B  (default lines=100)
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_logs(const crow::request& req)
{
    return read_log_file(fs::current_path() / "logs" / "salma.log", req);
}

// ---------------------------------------------------------------------------
// GET /api/logs/test?lines=N&offset=B  (default lines=100)
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_test_logs(const crow::request& req)
{
    return read_log_file(fs::current_path() / "test.log", req);
}

// ---------------------------------------------------------------------------
// POST /api/logs/clear
// ---------------------------------------------------------------------------

crow::response Mo2Controller::clear_logs()
{
    // Use Logger::clear_log() to coordinate truncation with the persistent
    // file handle, avoiding corrupted writes from concurrent log calls.
    auto& logger = mo2core::Logger::instance();
    if (logger.clear_log())
    {
        logger.log("[server] Cleared logs/salma.log");
        return json_response(200, {{"success", true}});
    }
    return json_response(500, {{"error", "Failed to clear salma.log"}});
}

// ---------------------------------------------------------------------------
// POST /api/logs/clear/test
// ---------------------------------------------------------------------------

crow::response Mo2Controller::clear_test_logs()
{
    auto log_path = fs::current_path() / "test.log";

    try
    {
        std::ofstream ofs(log_path, std::ios::trunc);
        if (!ofs)
        {
            return json_response(500, {{"error", "Failed to clear test.log"}});
        }
        mo2core::Logger::instance().log("[server] Cleared test.log");
        return json_response(200, {{"success", true}});
    }
    catch (const std::exception& ex)
    {
        return json_response(500,
                             {{"error", std::format("Failed to clear test.log: {}", ex.what())}});
    }
}

// ---------------------------------------------------------------------------
// POST /api/test/run   -- spawn test.py in background
// ---------------------------------------------------------------------------

crow::response Mo2Controller::run_tests(const crow::request& req)
{
#ifdef _WIN32
    std::lock_guard<std::mutex> lock(test_mutex_);

    // Check if already running
    if (test_running_.load())
    {
        if (test_process_)
        {
            DWORD exit_code = 0;
            if (GetExitCodeProcess(test_process_, &exit_code) && exit_code == STILL_ACTIVE)
            {
                return json_response(409, {{"error", "Tests are already running"}});
            }
            // Process finished -- clean up
            CloseHandle(test_process_);
            test_process_ = nullptr;
            test_running_.store(false);
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
        catch (...)
        {
        }
    }

    // Sanitize args: reject shell metacharacters to prevent command injection.
    // Double-quotes and semicolons are also blocked to prevent cmd.exe string breakout.
    for (char c : args)
    {
        if (c == '&' || c == '|' || c == '>' || c == '<' || c == '^' || c == '%' || c == '!' ||
            c == '`' || c == '"' || c == ';' || c == '(' || c == ')' || c == '\n' || c == '\r')
        {
            return json_response(400, {{"error", "Invalid characters in test arguments"}});
        }
    }

    auto py_path = fs::current_path() / "test.py";
    if (!fs::exists(py_path))
        return json_response(
            404, {{"error", std::format("test.py not found in {}", fs::current_path().string())}});

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
                             fs::current_path().string().c_str(),
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
    test_running_.store(true);

    mo2core::Logger::instance().log(
        std::format("[server] Test suite started (PID {})", pi.dwProcessId));
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

    if (!test_running_.load() || !test_process_)
    {
        return json_response(200, {{"running", false}});
    }

    DWORD exit_code = 0;
    if (GetExitCodeProcess(test_process_, &exit_code) && exit_code == STILL_ACTIVE)
    {
        return json_response(200, {{"running", true}});
    }

    // Process finished
    CloseHandle(test_process_);
    test_process_ = nullptr;
    test_running_.store(false);

    return json_response(200, {{"running", false}, {"exitCode", static_cast<int>(exit_code)}});
#else
    return json_response(200, {{"running", false}});
#endif
}

}  // namespace mo2server

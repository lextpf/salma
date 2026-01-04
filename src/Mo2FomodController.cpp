#include "Mo2Controller.h"
#include "Mo2Helpers.h"

#include "ConfigService.h"
#include "FomodInferenceService.h"
#include "Logger.h"
#include "Utils.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <format>
#include <fstream>
#include <functional>
#include <nlohmann/json.hpp>
#include <regex>
#include <vector>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2server
{

// ---------------------------------------------------------------------------
// Static helpers
// ---------------------------------------------------------------------------

// Parse [General] installationFile from MO2 meta.ini.
static std::string read_installation_file(const fs::path& meta_ini_path)
{
    std::ifstream ifs(meta_ini_path);
    if (!ifs)
    {
        return "";
    }

    bool in_general = false;
    bool first_line = true;
    std::string line;
    while (std::getline(ifs, line))
    {
        if (!line.empty() && line.back() == '\r')
        {
            line.pop_back();
        }

        // Strip UTF-8 BOM if present on the first line
        if (first_line)
        {
            first_line = false;
            if (line.size() >= 3 && static_cast<unsigned char>(line[0]) == 0xEF &&
                static_cast<unsigned char>(line[1]) == 0xBB &&
                static_cast<unsigned char>(line[2]) == 0xBF)
            {
                line = line.substr(3);
            }
        }

        auto trimmed = trim_copy(line);
        if (trimmed.empty() || trimmed[0] == ';' || trimmed[0] == '#')
        {
            continue;
        }

        if (trimmed.front() == '[' && trimmed.back() == ']')
        {
            auto section = mo2core::to_lower(trim_copy(trimmed.substr(1, trimmed.size() - 2)));
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

        auto key = mo2core::to_lower(trim_copy(trimmed.substr(0, eq)));
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

/// SAX callback that counts the number of objects in the top-level "steps"
/// array of a FOMOD JSON file without fully parsing the document.
struct StepCounter : nlohmann::json_sax<json>
{
    int depth = 0;
    bool in_steps = false;
    int steps_depth = 0;
    int& count;
    explicit StepCounter(int& c)
        : count(c)
    {
    }

    bool key(string_t& key) override
    {
        if (depth == 1 && key == "steps")
            in_steps = true;
        return true;
    }
    bool start_array(std::size_t) override
    {
        if (in_steps && !steps_depth)
            steps_depth = depth + 1;
        depth++;
        return true;
    }
    bool end_array() override
    {
        depth--;
        if (steps_depth && depth < steps_depth)
        {
            // Finished the steps array -- no need to parse further.
            // Reset state so a second array at the same depth won't
            // be mistaken for the steps array.
            in_steps = false;
            steps_depth = 0;
            return false;
        }
        return true;
    }
    bool start_object(std::size_t) override
    {
        if (steps_depth && depth == steps_depth)
            count++;
        depth++;
        return true;
    }
    bool end_object() override
    {
        depth--;
        return true;
    }
    // Required overrides that just continue parsing:
    bool null() override { return true; }
    bool boolean(bool) override { return true; }
    bool number_integer(number_integer_t) override { return true; }
    bool number_unsigned(number_unsigned_t) override { return true; }
    bool number_float(number_float_t, const string_t&) override { return true; }
    bool string(string_t&) override { return true; }
    bool binary(binary_t&) override { return true; }
    bool parse_error(std::size_t, const std::string&, const nlohmann::detail::exception&) override
    {
        return false;
    }
};

// Per-mod inference result for the scan job.
enum class ModResult
{
    ExistingSkip,
    ArchiveSkip,
    Inferred,
    NoFomod,
    Error
};

using InferStatusFn =
    std::function<std::string(int, const std::string&, const std::string&, const std::string&)>;

static ModResult process_single_mod(const fs::path& mod_folder,
                                    const fs::path& output_dir,
                                    const fs::path& mods_dir,
                                    int index,
                                    int total,
                                    const InferStatusFn& infer_status)
{
    auto& logger = mo2core::Logger::instance();
    const std::string mod_name = mod_folder.filename().string();
    const fs::path choices_file = output_dir / (mod_name + ".json");

    if (fs::exists(choices_file))
    {
        logger.log(infer_status(index, mod_name, "SKIP", "(exists)"));
        return ModResult::ExistingSkip;
    }

    const std::string archive_value = read_installation_file(mod_folder / "meta.ini");
    if (archive_value.empty())
    {
        logger.log(infer_status(index, mod_name, "SKIP", "(no archive)"));
        return ModResult::ArchiveSkip;
    }

    fs::path archive_path = resolve_archive_path(archive_value, mod_folder, mods_dir);
    if (archive_path.empty() || !fs::exists(archive_path))
    {
        logger.log(infer_status(index, mod_name, "SKIP", "(archive missing)"));
        return ModResult::ArchiveSkip;
    }

    logger.log(
        std::format(R"([infer] [{}/{}]   Archive: "{}")", index + 1, total, archive_path.string()));

    try
    {
        auto item_start = std::chrono::steady_clock::now();
        mo2core::FomodInferenceService service;
        std::string result = service.infer_selections(archive_path.string(), mod_folder.string());
        double elapsed_s =
            std::chrono::duration<double>(std::chrono::steady_clock::now() - item_start).count();

        // Warn if a single mod took excessively long (> 5 minutes)
        if (elapsed_s > 300.0)
        {
            logger.log_warning(
                std::format("[infer] Mod \"{}\" took {:.1f}s to infer (exceeded 5m threshold)",
                            mod_name,
                            elapsed_s));
        }

        if (result.empty())
        {
            logger.log(infer_status(
                index, mod_name, "NOT FOMOD", std::format("({:.1f}s): no FOMOD data", elapsed_s)));
            return ModResult::NoFomod;
        }

        auto parsed = json::parse(result, nullptr, false);
        if (parsed.is_discarded())
        {
            logger.log_error(infer_status(index, mod_name, "ERROR", "(invalid JSON returned)"));
            return ModResult::Error;
        }

        if (!parsed.contains("steps") || !parsed["steps"].is_array() || parsed["steps"].empty())
        {
            logger.log(infer_status(
                index, mod_name, "NO STEPS", std::format("({:.1f}s): no FOMOD steps", elapsed_s)));
            return ModResult::NoFomod;
        }

        std::ofstream ofs(choices_file);
        if (!ofs)
        {
            logger.log_error(
                infer_status(index,
                             mod_name,
                             "ERROR",
                             std::format("(failed to open {})", choices_file.string())));
            return ModResult::Error;
        }

        ofs << parsed.dump(2);
        ofs.flush();
        if (!ofs.good())
        {
            logger.log_error(
                infer_status(index,
                             mod_name,
                             "ERROR",
                             std::format("(failed to write {})", choices_file.string())));
            return ModResult::Error;
        }

        logger.log(infer_status(
            index,
            mod_name,
            "INFERRED",
            std::format("({:.1f}s): {} steps saved", elapsed_s, parsed["steps"].size())));
        return ModResult::Inferred;
    }
    catch (const std::exception& ex)
    {
        logger.log_error(infer_status(index, mod_name, "ERROR", std::format("({})", ex.what())));
        return ModResult::Error;
    }
}

static json run_fomod_scan_job(const fs::path& mods_dir,
                               const fs::path& output_dir,
                               const std::atomic<bool>& cancel_requested)
{
    auto& logger = mo2core::Logger::instance();

    std::vector<fs::path> mod_folders;
    try
    {
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
    }
    catch (const std::exception& ex)
    {
        logger.log_error(std::format("[infer] Failed to enumerate mods directory: {}", ex.what()));
        return json{{"success", false}, {"error", ex.what()}};
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
    InferStatusFn infer_status = [total](int index,
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
        if (cancel_requested.load())
        {
            logger.log("[infer] Scan cancelled by shutdown request");
            break;
        }

        auto result =
            process_single_mod(mod_folders[i], output_dir, mods_dir, i, total, infer_status);
        switch (result)
        {
            case ModResult::ExistingSkip:
                ++skipped_existing;
                break;
            case ModResult::ArchiveSkip:
            {
                // Distinguish between "no archive" and "archive missing" based on
                // whether meta.ini had an installationFile entry at all.
                const std::string archive_value =
                    read_installation_file(mod_folders[i] / "meta.ini");
                if (archive_value.empty())
                    ++no_archive;
                else
                    ++archive_missing;
                break;
            }
            case ModResult::Inferred:
                ++scanned;
                ++inferred;
                break;
            case ModResult::NoFomod:
                ++scanned;
                ++no_fomod;
                break;
            case ModResult::Error:
                ++scanned;
                ++errors;
                break;
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
// GET /api/mo2/fomods
// ---------------------------------------------------------------------------

crow::response Mo2Controller::list_fomods()
{
    {
        std::lock_guard<std::mutex> lock(cache_mutex_);
        if (fomods_cache_.is_fresh(std::chrono::seconds(5)))
        {
            return json_response(200, fomods_cache_.data);
        }
    }

    auto fomod_dir = ConfigService::instance().fomod_output_dir();
    if (fomod_dir.empty() || !fs::is_directory(fomod_dir))
    {
        return json_response(200, json::array());
    }

    json list = json::array();
    int parse_errors = 0;
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

        // Count steps via SAX callback to avoid parsing the entire JSON into memory.
        // The callback increments step_count for each element of the top-level "steps"
        // array and aborts parsing once the array ends, so the cost is proportional to
        // the number of steps, not the total file size.
        int step_count = 0;
        bool parse_ok = true;
        try
        {
            std::ifstream ifs(entry.path());
            StepCounter counter(step_count);
            json::sax_parse(ifs, &counter, json::input_format_t::json, false);
        }
        catch (const std::exception& ex)
        {
            parse_ok = false;
            ++parse_errors;
            mo2core::Logger::instance().log_error(std::format(
                "[server] Failed to read steps from {}: {}", entry.path().string(), ex.what()));
        }
        catch (...)
        {
            parse_ok = false;
            ++parse_errors;
            mo2core::Logger::instance().log_error(
                std::format("[server] Failed to count steps in {}", entry.path().string()));
        }

        json item = {
            {"name", name}, {"size", size}, {"modified", epoch}, {"stepCount", step_count}};
        if (!parse_ok)
        {
            item["parseError"] = true;
        }
        list.push_back(std::move(item));
    }

    if (parse_errors > 0)
    {
        mo2core::Logger::instance().log_warning(
            std::format("[server] {} FOMOD JSON(s) had parse errors during listing", parse_errors));
    }

    {
        std::lock_guard<std::mutex> lock(cache_mutex_);
        fomods_cache_.set(list);
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

    bool started = scan_job_.try_start(
        [this, mods_dir, output_dir]() -> ScanResult
        {
            json summary = run_fomod_scan_job(mods_dir, output_dir, scan_job_.cancel_token());

            {
                std::lock_guard<std::mutex> lock(cache_mutex_);
                fomods_cache_.invalidate();
                status_cache_.invalidate();
            }

            ScanResult r;
            r.success = summary.value("success", false);
            r.total_mod_folders = summary.value("totalModFolders", 0);
            r.archives_processed = summary.value("archivesProcessed", 0);
            r.choices_inferred = summary.value("choicesInferred", 0);
            r.no_fomod = summary.value("noFomod", 0);
            r.already_had_choices = summary.value("alreadyHadChoices", 0);
            r.no_archive_found = summary.value("noArchiveFound", 0);
            r.archive_missing = summary.value("archiveMissing", 0);
            r.errors = summary.value("errors", 0);
            r.duration_ms = summary.value("durationMs", 0LL);
            r.output_dir = summary.value("outputDir", std::string{});
            return r;
        });

    if (!started)
    {
        return json_response(409, {{"error", "FOMOD scan is already running"}});
    }

    return json_response(200, {{"success", true}, {"running", true}, {"started", true}});
}

// ---------------------------------------------------------------------------
// GET /api/mo2/fomods/scan/status
// ---------------------------------------------------------------------------

crow::response Mo2Controller::get_scan_status()
{
    json result = {{"running", scan_job_.is_running()}};

    scan_job_.read_result(
        [&](bool has_result, const ScanResult* r, const std::string& error)
        {
            if (!has_result || !r)
                return;
            result["success"] = r->success;
            result["totalModFolders"] = r->total_mod_folders;
            result["archivesProcessed"] = r->archives_processed;
            result["choicesInferred"] = r->choices_inferred;
            result["noFomod"] = r->no_fomod;
            result["alreadyHadChoices"] = r->already_had_choices;
            result["noArchiveFound"] = r->no_archive_found;
            result["archiveMissing"] = r->archive_missing;
            result["errors"] = r->errors;
            result["durationMs"] = r->duration_ms;
            result["outputDir"] = r->output_dir;
            if (!error.empty())
            {
                result["error"] = error;
            }
        });

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

    if (!mo2core::is_inside(fomod_dir, file_path))
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
        mo2core::Logger::instance().log_error(std::format(
            "[server] Failed to read FOMOD JSON {}: {}", file_path.string(), ex.what()));
        return json_response(500, {{"error", "Failed to read FOMOD JSON"}});
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

    if (!mo2core::is_inside(fomod_dir, file_path))
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
        mo2core::Logger::instance().log_error(
            std::format("[server] Failed to delete FOMOD JSON {}: {}", decoded, ex.what()));
        return json_response(500, {{"error", "Failed to delete FOMOD JSON"}});
    }
}

}  // namespace mo2server

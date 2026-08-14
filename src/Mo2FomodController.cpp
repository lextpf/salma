#include "Mo2Controller.hpp"
#include "Mo2Helpers.hpp"

#include "ConfigService.hpp"
#include "SalmaEngine.hpp"
#include "Logger.hpp"
#include "Utils.hpp"

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

// Mo2FomodController - the FOMOD scan job and the /api/mo2/fomods reads.
//
// The scan walks every mod folder under the configured MO2 mods directory and
// tries to recover the FOMOD choices the user originally made, writing one
// <mod name>.json per success into the FOMOD output directory. It runs on a
// BackgroundJob; only one scan may run at a time.
//
// Per-mod pipeline (process_single_mod), first match wins:
//
//   for each directory under <mods>      ("Salma FOMODs Output" is skipped)
//     |
//     +-- <output>/<mod>.json exists? --------yes--> ExistingSkip
//     +-- meta.ini [General] installationFile? -no--> ArchiveSkipNoValue
//     +-- SalmaEngine::resolve_mod_archive ----no--> ArchiveSkipMissing
//     +-- SalmaEngine::infer_selections
//           returns ""            -------------------> NoFomod
//           invalid JSON          -------------------> Error
//           no non-empty "steps"  -------------------> NoFomod
//           otherwise: inject_choice_metadata
//                      -> write <mod>.json.tmp
//                      -> rename over <mod>.json ----> Inferred
//
// The write is tmp-then-rename so a crash mid-write cannot leave a half-written
// choices file that a later scan would then treat as ExistingSkip.
//
// Each outcome increments a fixed set of counters in the summary JSON that
// run_fomod_scan_job returns and /api/mo2/fomods/scan/status reports. This
// mapping is the contract with the dashboard's summary panel:
//
//   outcome              counters incremented
//   -------------------  -------------------------------------
//   ExistingSkip         alreadyHadChoices
//   ArchiveSkipNoValue   noArchiveFound
//   ArchiveSkipMissing   archiveMissing
//   Inferred             archivesProcessed + choicesInferred
//   NoFomod              archivesProcessed + noFomod
//   Error                archivesProcessed + errors
//
// totalModFolders counts every folder considered, so the six counters sum to it
// only when the scan was not cancelled.
//
// Per-mod progress is reported as log lines, not as job state, and the
// dashboard parses those lines. infer_status builds each row as
// "<label> <dots> <status> <detail>", padding the dot run so the status word
// starts near column 64:
//
//   [infer] [7/312] SkyUI ............................ INFERRED (1.4s)
//
// web/src/progressBarParsing.tsx locates the status word with `\.{3,}`, so the
// dot run must never fall below 4 characters. infer_status holds that floor
// even when the label alone is longer than the column.

namespace mo2server
{

// ---------------------------------------------------------------------------
// Static helpers
// ---------------------------------------------------------------------------

// Parse [General] installationFile from MO2 meta.ini. Returns the value with
// surrounding single or double quotes removed, or "" when the file is absent,
// has no [General] section, or has no installationFile key. Section and key
// comparison is case-insensitive; the value is returned verbatim.
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

// Archive resolution lives in the engine DLL and is reached through
// `SalmaEngine::resolve_mod_archive` (the `resolveModArchive` C-API export), so
// the dashboard and the MO2 plugin run the same implementation.

// One SAX pass over a FOMOD choices JSON. It counts the objects in the
// top-level "steps" array and, on the way there, captures
// diagnostics.confidence.composite, diagnostics.confidence.band and
// diagnostics.exact_match, without materializing the document.
//
// One pass suffices because both nlohmann::json and the engine's writer store
// object members in sorted key order, so the top-level keys arrive as
// diagnostics, metadata, outputTree (with its outputTree* siblings),
// schema_version, steps. diagnostics always precedes steps.
//
// Each captured value has a paired present-flag, so the caller emits nothing
// for a JSON written before the diagnostics block existed.
//
// Keep this helper on plain `//` comments: doxide globs src/*.cpp and would
// otherwise publish a reference page for it next to the real controllers.
struct StepCounter : nlohmann::json_sax<json>
{
    int depth = 0;
    bool in_steps = false;
    int steps_depth = 0;
    int& count;

    // Out-parameters, each with its present-flag.
    double& confidence;
    std::string& band;
    bool& exact_match;
    bool& has_confidence;
    bool& has_band;
    bool& has_exact;

    bool arm_diag = false;  // the next object opened is the diagnostics value
    bool in_diag = false;
    int diag_depth = 0;
    bool arm_conf = false;  // the next object opened is the confidence value
    bool in_conf = false;
    int conf_depth = 0;

    enum class Pending : std::uint8_t
    {
        None,
        Composite,
        Band,
        ExactMatch
    };
    Pending pending = Pending::None;

    StepCounter(int& c,
                double& conf,
                std::string& bnd,
                bool& exact,
                bool& has_conf,
                bool& has_bnd,
                bool& has_exc)
        : count(c),
          confidence(conf),
          band(bnd),
          exact_match(exact),
          has_confidence(has_conf),
          has_band(has_bnd),
          has_exact(has_exc)
    {
    }

    bool key(string_t& key) override
    {
        if (depth == 1)
        {
            if (key == "steps")
            {
                in_steps = true;
            }
            else if (key == "diagnostics")
            {
                arm_diag = true;
            }
        }
        else if (in_diag && depth == diag_depth)
        {
            if (key == "confidence")
            {
                arm_conf = true;
            }
            else if (key == "exact_match")
            {
                pending = Pending::ExactMatch;
            }
        }
        else if (in_conf && depth == conf_depth)
        {
            if (key == "composite")
            {
                pending = Pending::Composite;
            }
            else if (key == "band")
            {
                pending = Pending::Band;
            }
        }
        return true;
    }
    bool start_array(std::size_t) override
    {
        if (in_steps && !steps_depth)
        {
            steps_depth = depth + 1;
        }
        // diagnostics and confidence are objects, never arrays. Drop any armed
        // flag so a stray array value cannot be mistaken for one of them.
        arm_diag = false;
        arm_conf = false;
        depth++;
        return true;
    }
    bool end_array() override
    {
        depth--;
        if (steps_depth && depth < steps_depth)
        {
            // The steps array is closed, so stop parsing. Reset the state so a
            // second array at the same depth cannot be taken for it.
            in_steps = false;
            steps_depth = 0;
            return false;
        }
        return true;
    }
    bool start_object(std::size_t) override
    {
        if (steps_depth && depth == steps_depth)
        {
            count++;
        }
        if (arm_diag)
        {
            in_diag = true;
            diag_depth = depth + 1;
            arm_diag = false;
        }
        else if (arm_conf && in_diag)
        {
            in_conf = true;
            conf_depth = depth + 1;
            arm_conf = false;
        }
        depth++;
        return true;
    }
    bool end_object() override
    {
        depth--;
        if (in_conf && depth < conf_depth)
        {
            in_conf = false;
        }
        if (in_diag && depth < diag_depth)
        {
            in_diag = false;
        }
        return true;
    }
    // Value callbacks. Each stores whatever `pending` armed, then keeps going.
    bool null() override { return true; }
    bool boolean(bool val) override
    {
        if (pending == Pending::ExactMatch)
        {
            exact_match = val;
            has_exact = true;
            pending = Pending::None;
        }
        return true;
    }
    bool number_integer(number_integer_t val) override
    {
        capture_number(static_cast<double>(val));
        return true;
    }
    bool number_unsigned(number_unsigned_t val) override
    {
        capture_number(static_cast<double>(val));
        return true;
    }
    bool number_float(number_float_t val, const string_t&) override
    {
        capture_number(val);
        return true;
    }
    bool string(string_t& val) override
    {
        if (pending == Pending::Band)
        {
            band = val;
            has_band = true;
            pending = Pending::None;
        }
        return true;
    }
    bool binary(binary_t&) override { return true; }
    bool parse_error(std::size_t, const std::string&, const nlohmann::detail::exception&) override
    {
        return false;
    }

private:
    void capture_number(double val)
    {
        if (pending == Pending::Composite)
        {
            confidence = val;
            has_confidence = true;
            pending = Pending::None;
        }
    }
};

// Parse a Nexus-style archive filename "Name-ModID-Version-FileID" and return
// {modid, fileid}, or two empty strings when the name does not follow the
// convention. The pair goes into the choices JSON metadata block, which lets
// the MO2 plugin look up choices by stable identifier instead of a fuzzy
// filename stem-prefix match.
//
// strip_nexus_suffix in InstallationController.cpp decodes the same convention
// with its own regex. The two are deliberately separate: that one needs only
// the stem, so it is greedy on the stem and restrictive on the version segment,
// while this one needs the two numeric ids, so it is lazy on the stem and
// unrestricted in the middle. They can disagree on a name carrying extra
// hyphen-digit groups. Change one and check the other.
static std::pair<std::string, std::string> parse_nexus_archive_name(const std::string& filename)
{
    static const std::regex kPattern(R"(^(.+?)-(\d+)-(.*)-(\d+)$)");
    std::smatch match;
    if (std::regex_match(filename, match, kPattern))
    {
        return {match[2].str(), match[4].str()};
    }
    return {};
}

// Format a system-clock time point as "YYYY-MM-DDTHH:MM:SSZ" UTC.
static std::string format_iso8601_utc(std::chrono::system_clock::time_point tp)
{
    auto t = std::chrono::system_clock::to_time_t(tp);
    std::tm tm_utc{};
#ifdef _WIN32
    gmtime_s(&tm_utc, &t);
#else
    gmtime_r(&t, &tm_utc);
#endif
    return std::format("{:04d}-{:02d}-{:02d}T{:02d}:{:02d}:{:02d}Z",
                       tm_utc.tm_year + 1900,
                       tm_utc.tm_mon + 1,
                       tm_utc.tm_mday,
                       tm_utc.tm_hour,
                       tm_utc.tm_min,
                       tm_utc.tm_sec);
}

// Add the `metadata` block to a parsed choices JSON before it is written. The
// consumer is _find_fomod_json in scripts/mo2-salma.py, which tries
// (modid, fileid) first, then (archive_size, archive_mtime), then an exact
// filename match, and only then a stem-prefix match. That last fallback is
// fragile, because two similarly named mods pick each other's choices file.
// Every field written here exists to keep the lookup off it.
//
// archive_mtime is whole POSIX seconds, because the Python side compares it
// against int(stat.st_mtime) with a one-second tolerance. archive_size is in
// bytes. Both become 0 when the archive cannot be stat'ed, which makes the
// fingerprint lookup miss and nothing worse.
static void inject_choice_metadata(json& parsed,
                                   const fs::path& archive_path,
                                   const std::string& mod_name)
{
    json metadata;
    metadata["module_name"] = mod_name;
    metadata["archive_path"] = archive_path.string();

    std::error_code ec;
    auto size = fs::file_size(archive_path, ec);
    metadata["archive_size"] = ec ? 0 : static_cast<uint64_t>(size);

    auto file_time = fs::last_write_time(archive_path, ec);
    if (!ec)
    {
        // file_clock and system_clock have different epochs on Windows, so
        // clock_cast bridges them. Without it the value would not be POSIX
        // seconds and the Python lookup could never match.
        auto sctp = std::chrono::clock_cast<std::chrono::system_clock>(file_time);
        metadata["archive_mtime"] =
            std::chrono::duration_cast<std::chrono::seconds>(sctp.time_since_epoch()).count();
    }
    else
    {
        metadata["archive_mtime"] = 0;
    }

    auto [modid, fileid] = parse_nexus_archive_name(archive_path.stem().string());
    metadata["modid"] = modid;
    metadata["fileid"] = fileid;
    metadata["scanned_at"] = format_iso8601_utc(std::chrono::system_clock::now());

    parsed["metadata"] = metadata;
}

// Per-mod inference result. Every value maps to a counter in the scan summary;
// see the table at the top of this file.
//
// The archive-skip case splits into ArchiveSkipNoValue and ArchiveSkipMissing
// so the caller can tell "no installationFile entry" from "entry set, archive
// gone" without re-parsing meta.ini. The dashboard reports them separately,
// because the two need different user action.
enum class ModResult
{
    ExistingSkip,
    ArchiveSkipNoValue,  // meta.ini absent or has no [General] installationFile entry
    ArchiveSkipMissing,  // meta.ini named an archive but the file was not found
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
        return ModResult::ArchiveSkipNoValue;
    }

    fs::path archive_path = mo2server::SalmaEngine::resolve_mod_archive(archive_value, mod_folder, mods_dir);
    if (archive_path.empty() || !fs::exists(archive_path))
    {
        logger.log(infer_status(index, mod_name, "SKIP", "(archive missing)"));
        return ModResult::ArchiveSkipMissing;
    }

    logger.log(
        std::format(R"([infer] [{}/{}]   Archive: "{}")", index + 1, total, archive_path.string()));

    try
    {
        auto item_start = std::chrono::steady_clock::now();
        std::string result = mo2server::SalmaEngine::infer_selections(archive_path.string(), mod_folder.string());
        double elapsed_s =
            std::chrono::duration<double>(std::chrono::steady_clock::now() - item_start).count();

        // A single mod past 5 minutes usually means the solver is thrashing on
        // a pathological installer. Flag it so the scan log names the culprit.
        if (elapsed_s > 300.0)
        {
            logger.log_warning(
                std::format("[infer] Mod \"{}\" took {:.1f}s to infer (exceeded 5m threshold)",
                            mod_name,
                            elapsed_s));
        }

        if (result.empty())
        {
            logger.log(
                infer_status(index, mod_name, "NOT FOMOD", std::format("({:.1f}s)", elapsed_s)));
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

        inject_choice_metadata(parsed, archive_path, mod_name);

        // Write to .tmp and rename over the target. A crash mid-write then
        // leaves no half-written file for a later scan to read as ExistingSkip.
        auto tmp_file = choices_file;
        tmp_file += ".tmp";
        std::ofstream ofs(tmp_file);
        if (!ofs)
        {
            logger.log_error(infer_status(
                index, mod_name, "ERROR", std::format("(failed to open {})", tmp_file.string())));
            return ModResult::Error;
        }

        ofs << parsed.dump(2);
        ofs.flush();
        if (!ofs.good())
        {
            logger.log_error(infer_status(
                index, mod_name, "ERROR", std::format("(failed to write {})", tmp_file.string())));
            std::error_code rm_ec;
            fs::remove(tmp_file, rm_ec);
            return ModResult::Error;
        }
        ofs.close();

        std::error_code rename_ec;
        fs::rename(tmp_file, choices_file, rename_ec);
        if (rename_ec)
        {
            logger.log_error(infer_status(index,
                                          mod_name,
                                          "ERROR",
                                          std::format("(failed to rename {} -> {}: {})",
                                                      tmp_file.string(),
                                                      choices_file.string(),
                                                      rename_ec.message())));
            std::error_code rm_ec;
            fs::remove(tmp_file, rm_ec);
            return ModResult::Error;
        }

        logger.log(infer_status(index, mod_name, "INFERRED", std::format("({:.1f}s)", elapsed_s)));
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

    // The column the status word is padded out to. Raising it pushes the status
    // past the width of a log pane and turns every scan row into an ellipsis;
    // 64 aligns the common case and still fits. The floor of 4 dots below is
    // load-bearing: the dashboard's scan-progress regex (progressBarParsing.tsx,
    // `\.{3,}`) finds the status word by the dot run.
    constexpr size_t kInferDotColumn = 64;
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
            case ModResult::ArchiveSkipNoValue:
                ++no_archive;
                break;
            case ModResult::ArchiveSkipMissing:
                ++archive_missing;
                break;
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

        // Counting through a SAX callback keeps peak memory constant in the file
        // size instead of proportional to it.
        //
        // Time is still linear in the bytes. The early abort in
        // StepCounter::end_array only skips what follows the steps array, and
        // "steps" sorts last among the top-level keys, so the lexer has already
        // walked the whole file by then. Listing a directory of large choices
        // JSONs is expensive; the cache above is what makes it acceptable.
        int step_count = 0;
        double confidence = 0.0;
        std::string band;
        bool exact_match = false;
        bool has_confidence = false;
        bool has_band = false;
        bool has_exact = false;
        bool parse_ok = true;
        try
        {
            std::ifstream ifs(entry.path());
            StepCounter counter(
                step_count, confidence, band, exact_match, has_confidence, has_band, has_exact);
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
        if (has_confidence)
        {
            item["confidence"] = confidence;
        }
        if (has_band)
        {
            item["confidenceBand"] = band;
        }
        if (has_exact)
        {
            item["exactMatch"] = exact_match;
        }
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
    // `running` is read outside read_result's mutex-held callback, unlike
    // InstallationController::handle_status. A poll landing between the worker
    // storing its result and clearing the running flag can therefore report
    // running=true next to a completed summary. That is acceptable here: the
    // dashboard polls on a timer, the next poll corrects it, and the scan
    // summary is advisory rather than a completion signal anything acts on. Do
    // not copy this relaxation into a status read whose result gates work.
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

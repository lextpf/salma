#include "FomodInferenceService.h"
#include "ArchiveService.h"
#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodForwardSimulator.h"
#include "FomodInferenceAtoms.h"
#include "FomodIR.h"
#include "FomodIRParser.h"
#include "FomodPropagator.h"
#include "Logger.h"
#include "Utils.h"

#include <algorithm>
#include <cassert>
#include <chrono>
#include <format>
#include <fstream>
#include <mutex>
#include <unordered_map>
#include <unordered_set>

namespace fs = std::filesystem;
using json = nlohmann::json;

namespace mo2core
{

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// scan_installed_files
// ---------------------------------------------------------------------------
std::unordered_map<std::string, uint64_t> FomodInferenceService::scan_installed_files(
    const fs::path& mod_path)
{
    std::unordered_map<std::string, uint64_t> files;
    if (!fs::exists(mod_path))
        return files;

    std::error_code iter_ec;
    for (const auto& entry : fs::recursive_directory_iterator(
             mod_path, fs::directory_options::skip_permission_denied, iter_ec))
    {
        try
        {
            if (!entry.is_regular_file())
            {
                continue;
            }
            auto rel = fs::relative(entry.path(), mod_path).string();
            std::error_code ec;
            auto sz = entry.file_size(ec);
            files[normalize_path(rel)] = ec ? 0 : static_cast<uint64_t>(sz);
        }
        catch (const std::exception& ex)
        {
            Logger::instance().log_warning(std::format(
                "[infer] Error processing entry in {}: {}", mod_path.string(), ex.what()));
        }
    }
    if (iter_ec)
    {
        Logger::instance().log_warning(
            std::format("[infer] Error iterating {}: {}", mod_path.string(), iter_ec.message()));
    }
    return files;
}

// ---------------------------------------------------------------------------
// try_fomod_plus_json
// ---------------------------------------------------------------------------
json FomodInferenceService::try_fomod_plus_json(const fs::path& mod_path)
{
    auto& logger = Logger::instance();
    auto meta_ini = mod_path / "meta.ini";
    if (!fs::exists(meta_ini))
        return {};

    std::ifstream ifs(meta_ini);
    if (!ifs)
        return {};

    bool in_settings = false;
    std::string line;
    int line_count = 0;
    while (std::getline(ifs, line))
    {
        if (++line_count > 10000)
        {
            Logger::instance().log_warning("[infer] meta.ini exceeds 10000 lines, aborting parse");
            return {};
        }
        auto trimmed = line;
        trimmed.erase(0, trimmed.find_first_not_of(" \t\r\n"));
        if (auto pos = trimmed.find_last_not_of(" \t\r\n"); pos != std::string::npos)
            trimmed.erase(pos + 1);
        else
            trimmed.clear();

        if (trimmed.empty())
            continue;
        if (trimmed.front() == '[')
        {
            in_settings = (to_lower(trimmed) == "[settings]");
            continue;
        }
        if (!in_settings)
            continue;

        auto eq_pos = trimmed.find('=');
        if (eq_pos == std::string::npos)
            continue;

        auto key = trimmed.substr(0, eq_pos);
        key.erase(0, key.find_first_not_of(" \t"));
        if (auto pos = key.find_last_not_of(" \t"); pos != std::string::npos)
            key.erase(pos + 1);
        else
            key.clear();

        if (to_lower(key) != "fomod plus/fomod")
            continue;

        auto value = trimmed.substr(eq_pos + 1);
        value.erase(0, value.find_first_not_of(" \t"));
        if (auto pos = value.find_last_not_of(" \t"); pos != std::string::npos)
            value.erase(pos + 1);
        else
            value.clear();

        if (value.size() >= 2 && value.front() == '"' && value.back() == '"')
            value = value.substr(1, value.size() - 2);

        if (value.empty() || value == "{}" || value == "\"{}\"")
        {
            logger.log("[infer] fomod-plus JSON found but empty - rejecting");
            return {};
        }

        try
        {
            auto j = json::parse(value);
            if (j.contains("steps") && j["steps"].is_array() && !j["steps"].empty())
            {
                logger.log("[infer] Using fomod-plus JSON from meta.ini (Tier 1)");
                return j;
            }
            logger.log("[infer] fomod-plus JSON has no steps - rejecting");
        }
        catch (const json::parse_error& e)
        {
            logger.log(std::format("[infer] Failed to parse fomod-plus JSON: {}", e.what()));
        }
        return {};
    }
    return {};
}

// ---------------------------------------------------------------------------
// Hash contested files: helper functions
// ---------------------------------------------------------------------------

namespace
{

struct ContestedResult
{
    std::unordered_set<std::string> contested_dests;
    std::unordered_set<std::string> entries_to_read;
};

/// Phase 1: Find destinations where multiple source files can produce the same
/// target path. Include all origins (Required/Plugin/Conditional) since
/// Required-vs-Plugin and Conditional-vs-Plugin collisions can otherwise look
/// "exact" on size alone.
ContestedResult find_contested_dests(const TargetTree& target,
                                     const AtomIndex& atom_index,
                                     const std::unordered_set<std::string>& excluded)
{
    ContestedResult result;

    for (const auto& [dest, target_file] : target)
    {
        if (excluded.count(dest))
            continue;
        auto it = atom_index.find(dest);
        if (it == atom_index.end())
            continue;

        // Count distinct candidate sources at this dest. Keep only atoms that
        // already size-match the target (or have unknown size), then disambiguate
        // by content hash.
        std::unordered_set<std::string> candidate_sources;
        for (const auto& a : it->second)
        {
            if (!a.source_path.empty() &&
                (a.file_size == 0 || target_file.size == 0 || a.file_size == target_file.size))
            {
                candidate_sources.insert(a.source_path);
            }
        }
        if (candidate_sources.size() > 1)
        {
            result.contested_dests.insert(dest);
            for (const auto& src : candidate_sources)
                result.entries_to_read.insert(src);
        }
    }

    return result;
}

struct HashResult
{
    std::unordered_map<std::string, uint64_t> source_hashes;
    std::unordered_map<std::string, uint64_t> source_sizes;
};

/// Build a signature string that uniquely identifies an archive file by its
/// canonical path, size, and last-write time.
std::string build_archive_signature(const std::string& archive_path)
{
    std::error_code ec;
    auto canon = fs::weakly_canonical(fs::path(archive_path), ec);
    auto normalized_path = normalize_path(ec ? archive_path : canon.string());
    uint64_t sz = 0;
    if (auto s = fs::file_size(archive_path, ec); !ec)
        sz = static_cast<uint64_t>(s);
    int64_t mtime = 0;
    if (auto wt = fs::last_write_time(archive_path, ec); !ec)
        mtime = wt.time_since_epoch().count();
    return std::format("{}|{}|{}", normalized_path, sz, mtime);
}

/// Phase 2: Check instance cache for hits, batch-read missing entries from the
/// archive, compute FNV-1a hashes, and update the cache.
HashResult fetch_entry_hashes(
    const std::string& archive_path,
    const std::unordered_set<std::string>& entries_to_read,
    std::mutex& cache_mutex,
    std::unordered_map<std::string, FomodInferenceService::CachedHash>& cache)
{
    auto& logger = Logger::instance();
    HashResult result;
    std::unordered_set<std::string> missing_entries;

    auto archive_sig = build_archive_signature(archive_path);
    int cache_hits = 0;
    {
        std::scoped_lock lock(cache_mutex);
        for (const auto& entry_path : entries_to_read)
        {
            auto key = std::format("{}\n{}", archive_sig, entry_path);
            auto it = cache.find(key);
            if (it != cache.end())
            {
                result.source_hashes[entry_path] = it->second.hash;
                result.source_sizes[entry_path] = it->second.size;
                cache_hits++;
            }
            else
            {
                missing_entries.insert(entry_path);
            }
        }
    }

    if (!missing_entries.empty())
    {
        ArchiveService archive_service;
        auto batch_data = archive_service.read_entries_batch(archive_path, missing_entries);

        std::vector<std::pair<std::string, FomodInferenceService::CachedHash>> new_entries;
        for (auto& [entry_path, data] : batch_data)
        {
            uint64_t h = fnv1a_hash(data.data(), data.size());
            uint64_t sz = static_cast<uint64_t>(data.size());
            result.source_hashes[entry_path] = h;
            result.source_sizes[entry_path] = sz;

            auto key = std::format("{}\n{}", archive_sig, entry_path);
            new_entries.emplace_back(std::move(key), FomodInferenceService::CachedHash{h, sz});
        }
        {
            std::scoped_lock lock(cache_mutex);
            for (auto& [key, value] : new_entries)
            {
                cache[key] = value;
            }
        }
    }
    logger.log(std::format(
        "[infer] Contested hash cache: hits={}, misses={}", cache_hits, missing_entries.size()));

    return result;
}

/// Phase 3: Update atoms in atom_index and ExpandedAtoms with the computed
/// hashes. Also hash installed files at contested destinations.
void apply_entry_hashes(AtomIndex& atom_index,
                        ExpandedAtoms& atoms,
                        TargetTree& target,
                        const std::unordered_set<std::string>& contested_dests,
                        const HashResult& hashes,
                        const fs::path& mod_path)
{
    auto& logger = Logger::instance();

    assert(hashes.source_hashes.size() == hashes.source_sizes.size() &&
           "source_hashes and source_sizes must have the same number of entries");

    // Update atoms in the index with hashes
    for (auto& [dest, atom_vec] : atom_index)
    {
        if (!contested_dests.count(dest))
            continue;
        for (auto& a : atom_vec)
        {
            auto sh = hashes.source_hashes.find(a.source_path);
            if (sh != hashes.source_hashes.end())
            {
                a.content_hash = sh->second;
                auto ss = hashes.source_sizes.find(a.source_path);
                if (ss != hashes.source_sizes.end())
                    a.file_size = ss->second;
            }
        }
    }

    // Also update atoms in the ExpandedAtoms struct
    atoms.for_each(
        [&](FomodAtom& a)
        {
            auto sh = hashes.source_hashes.find(a.source_path);
            if (sh != hashes.source_hashes.end())
            {
                a.content_hash = sh->second;
                auto ss = hashes.source_sizes.find(a.source_path);
                if (ss != hashes.source_sizes.end())
                    a.file_size = ss->second;
            }
        });

    // Hash installed files at contested dests
    for (const auto& dest : contested_dests)
    {
        auto it = target.find(dest);
        if (it == target.end())
            continue;

        auto full_path = mod_path / dest;
        std::error_code ec;
        auto sz = fs::file_size(full_path, ec);
        if (ec)
            continue;

        static constexpr uintmax_t kMaxHashFileSize = 256 * 1024 * 1024;  // 256 MiB
        if (sz > kMaxHashFileSize)
        {
            logger.log_warning(std::format(
                "[infer] Skipping oversized file for hashing ({} bytes): {}", sz, dest));
            continue;
        }

        std::ifstream ifs(full_path, std::ios::binary);
        if (!ifs)
            continue;

        // Read entire file and hash
        try
        {
            auto alloc_size = static_cast<size_t>(std::min(sz, kMaxHashFileSize));
            std::vector<char> buf(alloc_size);
            ifs.read(buf.data(), static_cast<std::streamsize>(alloc_size));
            auto bytes_read = static_cast<size_t>(ifs.gcount());

            it->second.hash = fnv1a_hash(buf.data(), bytes_read);
            it->second.size = static_cast<uint64_t>(bytes_read);
        }
        catch (const std::bad_alloc&)
        {
            logger.log_warning(
                std::format("[infer] Failed to allocate {} bytes for hashing: {}", sz, dest));
        }
    }
}

}  // anonymous namespace

// ---------------------------------------------------------------------------
// Hash contested files (both target and atoms) for disambiguation
// ---------------------------------------------------------------------------

void FomodInferenceService::hash_contested_files(TargetTree& target,
                                                 ExpandedAtoms& atoms,
                                                 AtomIndex& atom_index,
                                                 const fs::path& mod_path,
                                                 const std::string& archive_path,
                                                 const std::unordered_set<std::string>& excluded)
{
    auto& logger = Logger::instance();

    // Prevent unbounded memory growth in long-running server processes.
    // Both the size check and clear are under the same lock to avoid
    // a race where two threads both pass the check and both clear.
    {
        std::lock_guard<std::mutex> guard(cache_mutex_);
        if (archive_entry_hash_cache_.size() > kMaxCacheEntries)
        {
            logger.log(
                std::format("[infer] Hash cache exceeded {} entries, clearing", kMaxCacheEntries));
            archive_entry_hash_cache_.clear();
        }
    }

    // Phase 1: Find contested destinations
    auto [contested_dests, entries_to_read] = find_contested_dests(target, atom_index, excluded);

    if (contested_dests.empty())
        return;

    logger.log(std::format("[infer] Hashing {} contested dests ({} archive entries)",
                           contested_dests.size(),
                           entries_to_read.size()));

    // Phase 2: Fetch entry hashes (cache lookup + archive read)
    auto hashes =
        fetch_entry_hashes(archive_path, entries_to_read, cache_mutex_, archive_entry_hash_cache_);

    // Phase 3: Apply hashes to atoms and target files
    apply_entry_hashes(atom_index, atoms, target, contested_dests, hashes, mod_path);
}

// ---------------------------------------------------------------------------
// compute_overrides: pre-compute conditional and step visibility overrides
// ---------------------------------------------------------------------------

InferenceOverrides FomodInferenceService::compute_overrides(
    const FomodInstaller& installer,
    const ExpandedAtoms& atoms,
    const AtomIndex& atom_index,
    const TargetTree& target,
    const std::unordered_set<std::string>& excluded)
{
    auto& logger = Logger::instance();

    InferenceOverrides overrides;
    overrides.conditional_active.resize(installer.conditional_patterns.size(),
                                        ExternalConditionOverride::Unknown);
    std::unordered_map<std::string, std::unordered_set<int>> cond_only_dest_patterns;
    for (const auto& [dest, atoms_for_dest] : atom_index)
    {
        if (excluded.count(dest) || !target.count(dest))
            continue;

        bool only_conditional = true;
        for (const auto& atom : atoms_for_dest)
        {
            if (atom.origin != FomodAtom::Origin::Conditional)
            {
                only_conditional = false;
                break;
            }
        }
        if (!only_conditional)
            continue;

        auto& producers = cond_only_dest_patterns[dest];
        for (const auto& atom : atoms_for_dest)
        {
            if (atom.conditional_index >= 0)
                producers.insert(atom.conditional_index);
        }
    }

    int cond_unique_forced = 0;
    int cond_ambiguous_skipped = 0;
    for (size_t ci = 0; ci < atoms.per_conditional.size(); ++ci)
    {
        bool has_unique_cond_only_hit = false;
        for (const auto& atom : atoms.per_conditional[ci])
        {
            if (excluded.count(atom.dest_path))
                continue;
            if (!target.count(atom.dest_path))
                continue;

            auto it = cond_only_dest_patterns.find(atom.dest_path);
            if (it == cond_only_dest_patterns.end())
                continue;
            if (it->second.empty())
                continue;
            if (it->second.size() == 1 && it->second.count(static_cast<int>(ci)) > 0)
            {
                has_unique_cond_only_hit = true;
                break;
            }
            cond_ambiguous_skipped++;
        }

        if (has_unique_cond_only_hit)
        {
            overrides.conditional_active[ci] = ExternalConditionOverride::ForceTrue;
            cond_unique_forced++;
        }
    }
    logger.log(
        std::format("[infer] Step 7b conditional evidence: unique_forced={}, "
                    "ambiguous_skipped={}",
                    cond_unique_forced,
                    cond_ambiguous_skipped));
    // Pre-compute step visibility: a step is visible if any of its plugins'
    // atoms have a dest in the target tree (evidence that the step was selected)
    overrides.step_visible.resize(installer.steps.size(), ExternalConditionOverride::Unknown);
    {
        int flat_idx_sv = 0;
        for (size_t si = 0; si < installer.steps.size(); ++si)
        {
            bool has_target_hit = false;
            for (const auto& group : installer.steps[si].groups)
            {
                for (size_t pi = 0; pi < group.plugins.size(); ++pi)
                {
                    if (!has_target_hit && flat_idx_sv < static_cast<int>(atoms.per_plugin.size()))
                    {
                        for (const auto& atom : atoms.per_plugin[flat_idx_sv])
                        {
                            if (!excluded.count(atom.dest_path) && target.count(atom.dest_path))
                            {
                                has_target_hit = true;
                                break;
                            }
                        }
                    }
                    else if (flat_idx_sv >= static_cast<int>(atoms.per_plugin.size()))
                    {
                        Logger::instance().log_warning(
                            std::format("[infer] compute_overrides: flat_idx {} out of range ({}), "
                                        "possible IR/atom desync",
                                        flat_idx_sv,
                                        atoms.per_plugin.size()));
                    }
                    flat_idx_sv++;
                }
            }
            if (has_target_hit)
                overrides.step_visible[si] = ExternalConditionOverride::ForceTrue;
        }
    }

    {
        int cond_true = 0;
        int cond_false = 0;
        int cond_unknown = 0;
        for (auto mode : overrides.conditional_active)
        {
            if (mode == ExternalConditionOverride::ForceTrue)
                cond_true++;
            else if (mode == ExternalConditionOverride::ForceFalse)
                cond_false++;
            else
                cond_unknown++;
        }

        int step_true = 0;
        int step_false = 0;
        int step_unknown = 0;
        for (auto mode : overrides.step_visible)
        {
            if (mode == ExternalConditionOverride::ForceTrue)
                step_true++;
            else if (mode == ExternalConditionOverride::ForceFalse)
                step_false++;
            else
                step_unknown++;
        }

        logger.log(
            std::format("[infer] Step 7b overrides: cond true/false/unknown={}/{}/{}, "
                        "steps true/false/unknown={}/{}/{}",
                        cond_true,
                        cond_false,
                        cond_unknown,
                        step_true,
                        step_false,
                        step_unknown));
    }

    return overrides;
}

// ---------------------------------------------------------------------------
// infer_selections: main entry point
// ---------------------------------------------------------------------------

std::string FomodInferenceService::infer_selections(const std::string& archive_path,
                                                    const std::string& mod_path)
{
    using clock = std::chrono::steady_clock;
    auto& logger = Logger::instance();
    auto t_total = clock::now();

    auto ms_since = [](auto start)
    { return std::chrono::duration_cast<std::chrono::milliseconds>(clock::now() - start).count(); };

    auto archive_ext = fs::path(archive_path).extension().string();
    std::error_code size_ec;
    auto archive_size_raw = fs::file_size(archive_path, size_ec);
    double archive_size_mb =
        size_ec ? 0.0 : static_cast<double>(archive_size_raw) / (1024.0 * 1024.0);

    logger.log("[infer] ========================================");
    logger.log(std::format(
        R"([infer] Archive: "{}" ({:.1f} MB, {}))", archive_path, archive_size_mb, archive_ext));
    logger.log(std::format(R"([infer] Mod path: "{}")", mod_path));
    logger.log("[infer] 0/9 Starting inference");

    if (!fs::exists(archive_path))
    {
        logger.log_error("[infer] Archive not found: " + archive_path);
        return "";
    }
    if (!fs::exists(mod_path))
    {
        logger.log_error("[infer] Mod path not found: " + mod_path);
        return "";
    }

    // Tier 1: Check for fomod-plus data in meta.ini
    auto t_step = clock::now();
    auto fomod_plus = try_fomod_plus_json(fs::path(mod_path));
    if (!fomod_plus.is_null() && !fomod_plus.empty())
    {
        logger.log(std::format("[infer] Tier 1 hit: fomod-plus JSON ({}ms)", ms_since(t_step)));
        return fomod_plus.dump(2);
    }
    logger.log(std::format("[infer] Tier 1 miss: no fomod-plus data ({}ms)", ms_since(t_step)));

    try
    {
        InferenceContext ctx;
        ArchiveService archive_service;

        // Step 1: List archive entries with sizes
        logger.log("[infer] 1/9 Listing archive entries");
        t_step = clock::now();
        ctx.listing = archive_service.list_entries_with_sizes(archive_path);
        auto& archive_entries = ctx.listing.paths;
        auto& entry_sizes = ctx.listing.sizes;
        ctx.t_list = ms_since(t_step);
        logger.log(std::format("[infer] Step 1 list_entries: {} entries, {} sizes ({}ms)",
                               archive_entries.size(),
                               entry_sizes.size(),
                               ctx.t_list));

        // Build sorted normalized entry index for O(log N) prefix lookups
        // Also build normalized sizes map keyed by normalized path
        ctx.sorted_norm_entries.reserve(archive_entries.size());
        for (const auto& entry : archive_entries)
        {
            if (entry.ends_with("/") || entry.ends_with("\\"))
                continue;
            auto norm = normalize_path(entry);
            ctx.sorted_norm_entries.push_back(norm);
            // Look up size using the original archive path (how listing.sizes is keyed)
            auto sz_it = entry_sizes.find(entry);
            if (sz_it != entry_sizes.end())
                ctx.norm_entry_sizes[norm] = sz_it->second;
        }
        std::sort(ctx.sorted_norm_entries.begin(), ctx.sorted_norm_entries.end());

        // Step 2: Find the FOMOD ModuleConfig entry (prefer shallowest path)
        logger.log("[infer] 2/9 Finding FOMOD config");
        constexpr std::string_view module_cfg_suffix = "fomod/moduleconfig.xml";
        size_t best_depth = static_cast<size_t>(-1);
        for (const auto& entry : archive_entries)
        {
            auto norm = normalize_path(entry);
            bool is_candidate = (norm == module_cfg_suffix) ||
                                (norm.size() > module_cfg_suffix.size() &&
                                 norm.ends_with(std::string("/") + std::string(module_cfg_suffix)));
            if (!is_candidate)
                continue;

            size_t depth = static_cast<size_t>(std::count(norm.begin(), norm.end(), '/'));
            if (depth < best_depth ||
                (depth == best_depth &&
                 (ctx.xml_entry_norm.empty() || norm.size() < ctx.xml_entry_norm.size())))
            {
                ctx.xml_entry_norm = norm;
                best_depth = depth;
            }
        }

        if (ctx.xml_entry_norm.empty())
        {
            logger.log(std::format("[infer] Not a FOMOD mod, total: {}ms", ms_since(t_total)));
            return "";
        }

        size_t suffix_pos = ctx.xml_entry_norm.size() - module_cfg_suffix.size();
        if (suffix_pos > 0 && ctx.xml_entry_norm[suffix_pos - 1] == '/')
            suffix_pos--;
        ctx.fomod_prefix = (suffix_pos > 0) ? ctx.xml_entry_norm.substr(0, suffix_pos) : "";

        logger.log(std::format("[infer] Step 2 found XML: \"{}\" (prefix: \"{}\")",
                               ctx.xml_entry_norm,
                               ctx.fomod_prefix));

        // Step 3: Read ModuleConfig.xml into memory
        logger.log("[infer] 3/9 Reading ModuleConfig.xml");
        t_step = clock::now();
        std::unordered_set<std::string> xml_set{ctx.xml_entry_norm};
        auto xml_data = archive_service.read_entries_batch(archive_path, xml_set);

        if (xml_data.find(ctx.xml_entry_norm) == xml_data.end())
        {
            logger.log_error("[infer] Failed to read ModuleConfig.xml from archive");
            return "";
        }
        const auto& xml_bytes = xml_data[ctx.xml_entry_norm];
        auto t_read_xml = ms_since(t_step);
        logger.log(
            std::format("[infer] Step 3 read XML: {} bytes ({}ms)", xml_bytes.size(), t_read_xml));

        // Step 4: Parse XML and build IR
        logger.log("[infer] 4/9 Parsing FOMOD XML");
        t_step = clock::now();
        pugi::xml_document doc;
        auto xml_result = doc.load_buffer(xml_bytes.data(), xml_bytes.size());
        if (!xml_result)
        {
            logger.log_error(std::format("[infer] XML parse failed: {}", xml_result.description()));
            return "";
        }

        ctx.installer = FomodIRParser::parse(doc, ctx.fomod_prefix);
        logger.log(std::format("[infer] Step 4 parse IR: {} steps, {} cond patterns ({}ms)",
                               ctx.installer.steps.size(),
                               ctx.installer.conditional_patterns.size(),
                               ms_since(t_step)));

        // Step 5: Expand atoms
        logger.log("[infer] 5/9 Expanding file atoms");
        t_step = clock::now();
        ctx.atoms = expand_all_atoms(ctx.installer, ctx.sorted_norm_entries, ctx.norm_entry_sizes);

        int total_atoms = 0;
        ctx.atoms.for_each([&](const FomodAtom&) { ++total_atoms; });

        ctx.atom_index = build_atom_index(ctx.atoms);
        ctx.excluded = compute_excluded_dests(ctx.atom_index);
        logger.log(
            std::format("[infer] Step 5 expand atoms: {} total, {} dests, {} excluded ({}ms)",
                        total_atoms,
                        ctx.atom_index.size(),
                        ctx.excluded.size(),
                        ms_since(t_step)));

        // Step 6: Build target tree
        logger.log("[infer] 6/9 Scanning installed files");
        t_step = clock::now();
        ctx.installed = scan_installed_files(fs::path(mod_path));
        ctx.target = build_target_tree(ctx.installed, ctx.atom_index, ctx.excluded);
        ctx.t_scan = ms_since(t_step);
        logger.log(std::format(
            "[infer] Step 6 target tree: {} files ({}ms)", ctx.target.size(), ctx.t_scan));

        // Step 7: Hash contested files for disambiguation
        logger.log("[infer] 7/9 Hashing contested files");
        t_step = clock::now();
        hash_contested_files(
            ctx.target, ctx.atoms, ctx.atom_index, fs::path(mod_path), archive_path, ctx.excluded);
        logger.log(std::format("[infer] Step 7 hash contested ({}ms)", ms_since(t_step)));

        // Step 7b: Pre-compute which conditional patterns have target-only files
        ctx.overrides =
            compute_overrides(ctx.installer, ctx.atoms, ctx.atom_index, ctx.target, ctx.excluded);

        // Step 7c: Constraint propagation pre-pass
        t_step = clock::now();
        ctx.propagation = propagate(ctx.installer,
                                    ctx.atoms,
                                    ctx.atom_index,
                                    ctx.target,
                                    ctx.excluded,
                                    ctx.overrides,
                                    nullptr);
        int total_groups = 0;
        for (const auto& s : ctx.installer.steps)
            total_groups += static_cast<int>(s.groups.size());
        logger.log(std::format(
            "[infer] Step 7c propagate: resolved={}/{} groups, fully_resolved={} ({}ms)",
            ctx.propagation.resolved_groups.size(),
            total_groups,
            ctx.propagation.fully_resolved,
            ms_since(t_step)));

        // Step 8: CSP solve (with propagation-narrowed domains)
        logger.log("[infer] 8/9 Solving (this may take a while)");
        t_step = clock::now();
        auto result =
            solve_fomod_csp(ctx.installer,
                            ctx.atoms,
                            ctx.atom_index,
                            ctx.target,
                            ctx.excluded,
                            &ctx.overrides,
                            ctx.propagation.resolved_groups.empty() ? nullptr : &ctx.propagation);
        ctx.t_solve = ms_since(t_step);
        logger.log(std::format("[infer] Step 8 solve: {} nodes, exact={} ({}ms)",
                               result.nodes_explored,
                               result.exact_match,
                               ctx.t_solve));

        // Step 9: Assemble JSON
        logger.log("[infer] 9/9 Assembling result");
        t_step = clock::now();
        auto json_result = assemble_json(ctx.installer, result);
        auto result_str = json_result.dump(2);
        logger.log(std::format(
            "[infer] Step 9 assemble JSON: {} bytes ({}ms)", result_str.size(), ms_since(t_step)));

        auto total_ms = ms_since(t_total);
        logger.log(std::format("[infer] DONE total={}ms | list={}ms scan={}ms solve={}ms",
                               total_ms,
                               ctx.t_list,
                               ctx.t_scan,
                               ctx.t_solve));
        logger.log("[infer] ========================================");

        return result_str;
    }
    catch (const std::exception& e)
    {
        logger.log_error(std::format("[infer] Error after {}ms: {}", ms_since(t_total), e.what()));
        return "";
    }
}

}  // namespace mo2core

#include "FomodInferenceService.h"
#include "ArchiveService.h"
#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodForwardSimulator.h"
#include "FomodIR.h"
#include "FomodIRParser.h"
#include "FomodPropagator.h"
#include "Logger.h"
#include "Utils.h"

#include <algorithm>
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
std::unordered_set<std::string> FomodInferenceService::scan_installed_files(
    const fs::path& mod_path)
{
    std::unordered_set<std::string> files;
    if (!fs::exists(mod_path))
        return files;

    for (const auto& entry : fs::recursive_directory_iterator(mod_path))
    {
        if (!entry.is_regular_file())
            continue;
        auto rel = fs::relative(entry.path(), mod_path).string();
        files.insert(normalize_path(rel));
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
    while (std::getline(ifs, line))
    {
        auto trimmed = line;
        trimmed.erase(0, trimmed.find_first_not_of(" \t\r\n"));
        trimmed.erase(trimmed.find_last_not_of(" \t\r\n") + 1);

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
        key.erase(key.find_last_not_of(" \t") + 1);

        if (to_lower(key) != "fomod plus/fomod")
            continue;

        auto value = trimmed.substr(eq_pos + 1);
        value.erase(0, value.find_first_not_of(" \t"));
        value.erase(value.find_last_not_of(" \t") + 1);

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
// Atom expansion: resolve IR file entries into concrete atoms
// ---------------------------------------------------------------------------

static void expand_entry(const FomodFileEntry& entry,
                         const std::vector<std::string>& sorted_entries,
                         const std::unordered_map<std::string, uint64_t>& entry_sizes,
                         int doc_order,
                         FomodAtom::Origin origin,
                         int plugin_idx,
                         int cond_idx,
                         std::vector<FomodAtom>& out)
{
    if (entry.is_folder)
    {
        auto prefix = entry.source;
        if (!prefix.ends_with("/"))
            prefix += "/";

        auto it = std::lower_bound(sorted_entries.begin(), sorted_entries.end(), prefix);
        while (it != sorted_entries.end() && it->starts_with(prefix))
        {
            auto rel = it->substr(prefix.size());
            auto dest = entry.destination.empty() ? rel : (entry.destination + "/" + rel);

            FomodAtom atom;
            atom.source_path = *it;
            atom.dest_path = normalize_path(dest);
            atom.priority = entry.priority;
            atom.document_order = doc_order;
            atom.origin = origin;
            atom.plugin_index = plugin_idx;
            atom.conditional_index = cond_idx;
            atom.always_install = entry.always_install;
            atom.install_if_usable = entry.install_if_usable;

            auto sz = entry_sizes.find(*it);
            if (sz != entry_sizes.end())
                atom.file_size = sz->second;

            out.push_back(std::move(atom));
            ++it;
        }
    }
    else
    {
        FomodAtom atom;
        atom.source_path = entry.source;
        atom.dest_path = entry.destination;
        atom.priority = entry.priority;
        atom.document_order = doc_order;
        atom.origin = origin;
        atom.plugin_index = plugin_idx;
        atom.conditional_index = cond_idx;
        atom.always_install = entry.always_install;
        atom.install_if_usable = entry.install_if_usable;

        auto sz = entry_sizes.find(entry.source);
        if (sz != entry_sizes.end())
            atom.file_size = sz->second;

        out.push_back(std::move(atom));
    }
}

static ExpandedAtoms expand_all_atoms(const FomodInstaller& installer,
                                      const std::vector<std::string>& sorted_entries,
                                      const std::unordered_map<std::string, uint64_t>& entry_sizes)
{
    ExpandedAtoms result;
    int doc_order = 0;

    // Required files
    for (const auto& entry : installer.required_files)
    {
        expand_entry(entry,
                     sorted_entries,
                     entry_sizes,
                     doc_order++,
                     FomodAtom::Origin::Required,
                     -1,
                     -1,
                     result.required);
    }

    // Count total plugins
    result.per_plugin.resize(total_flat_plugins(installer));

    // Pass 1: Normal (non-always) plugin file entries
    int flat_idx = 0;
    for (const auto& step : installer.steps)
    {
        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                for (const auto& entry : plugin.files)
                {
                    if (!entry.always_install && !entry.install_if_usable)
                    {
                        expand_entry(entry,
                                     sorted_entries,
                                     entry_sizes,
                                     doc_order++,
                                     FomodAtom::Origin::Plugin,
                                     flat_idx,
                                     -1,
                                     result.per_plugin[flat_idx]);
                    }
                }
                flat_idx++;
            }
        }
    }

    // Pass 2: Always-install and installIfUsable entries (higher doc_order)
    flat_idx = 0;
    for (const auto& step : installer.steps)
    {
        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                for (const auto& entry : plugin.files)
                {
                    if (entry.always_install || entry.install_if_usable)
                    {
                        expand_entry(entry,
                                     sorted_entries,
                                     entry_sizes,
                                     doc_order++,
                                     FomodAtom::Origin::Plugin,
                                     flat_idx,
                                     -1,
                                     result.per_plugin[flat_idx]);
                    }
                }
                flat_idx++;
            }
        }
    }

    // Conditional patterns
    result.per_conditional.resize(installer.conditional_patterns.size());
    for (size_t ci = 0; ci < installer.conditional_patterns.size(); ++ci)
    {
        for (const auto& entry : installer.conditional_patterns[ci].files)
        {
            expand_entry(entry,
                         sorted_entries,
                         entry_sizes,
                         doc_order++,
                         FomodAtom::Origin::Conditional,
                         -1,
                         static_cast<int>(ci),
                         result.per_conditional[ci]);
        }
    }

    return result;
}

// ---------------------------------------------------------------------------
// Build atom index and excluded dests
// ---------------------------------------------------------------------------

static AtomIndex build_atom_index(const ExpandedAtoms& atoms)
{
    AtomIndex index;
    atoms.for_each([&](const FomodAtom& a) { index[a.dest_path].push_back(a); });
    return index;
}

static std::unordered_set<std::string> compute_excluded_dests(const AtomIndex& atom_index)
{
    std::unordered_set<std::string> excluded;
    for (const auto& [dest, atoms] : atom_index)
    {
        bool all_auto = true;
        bool has_conditional = false;
        std::unordered_set<std::string> sources;
        for (const auto& a : atoms)
        {
            sources.insert(a.source_path);
            if (a.origin == FomodAtom::Origin::Conditional)
            {
                has_conditional = true;
            }
            else if (a.origin == FomodAtom::Origin::Plugin && !a.always_install &&
                     !a.install_if_usable)
            {
                all_auto = false;
            }
        }
        // Don't exclude conditional destinations - which conditionals fire depends
        // on flags, which depend on plugin selections. They carry solver signal.
        if (has_conditional)
            continue;
        // Exclude only if all atoms are auto-installed AND all from same source
        if (all_auto && sources.size() <= 1)
            excluded.insert(dest);
    }
    return excluded;
}

// ---------------------------------------------------------------------------
// Build target tree from installed files
// ---------------------------------------------------------------------------

static TargetTree build_target_tree(const std::unordered_set<std::string>& installed_files,
                                    const fs::path& mod_path,
                                    const AtomIndex& atom_index,
                                    const std::unordered_set<std::string>& excluded)
{
    TargetTree target;
    for (const auto& rel_path : installed_files)
    {
        // Skip MO2 metadata files that are never part of FOMOD installations
        if (rel_path == "meta.ini")
            continue;

        TargetFile tf;
        auto full_path = mod_path / rel_path;
        std::error_code ec;
        auto sz = fs::file_size(full_path, ec);
        if (!ec)
            tf.size = static_cast<uint64_t>(sz);
        target[rel_path] = tf;
    }
    return target;
}

// ---------------------------------------------------------------------------
// Hash contested files (both target and atoms) for disambiguation
// ---------------------------------------------------------------------------

static void hash_contested_files(
    TargetTree& target,
    ExpandedAtoms& atoms,
    AtomIndex& atom_index,
    const fs::path& mod_path,
    const std::string& archive_path,
    const std::unordered_set<std::string>& excluded,
    std::mutex& cache_mutex,
    std::unordered_map<std::string, FomodInferenceService::CachedHash>& archive_entry_hash_cache)
{
    auto& logger = Logger::instance();

    // Prevent unbounded memory growth in long-running server processes
    {
        std::lock_guard<std::mutex> guard(cache_mutex);
        if (archive_entry_hash_cache.size() > FomodInferenceService::kMaxCacheEntries)
        {
            logger.log("[infer] Hash cache exceeded limit, clearing");
            archive_entry_hash_cache.clear();
        }
    }

    auto build_archive_signature = [](const std::string& ap) -> std::string
    {
        std::error_code ec;
        auto canon = fs::weakly_canonical(fs::path(ap), ec);
        auto normalized_path = normalize_path(ec ? ap : canon.string());
        uint64_t sz = 0;
        if (auto s = fs::file_size(ap, ec); !ec)
            sz = static_cast<uint64_t>(s);
        int64_t mtime = 0;
        if (auto wt = fs::last_write_time(ap, ec); !ec)
            mtime = wt.time_since_epoch().count();
        return std::format("{}|{}|{}", normalized_path, sz, mtime);
    };

    // Find dests where multiple source files can produce the same target path.
    // Include all origins (Required/Plugin/Conditional): Required-vs-Plugin and
    // Conditional-vs-Plugin collisions can otherwise look "exact" on size alone.
    std::unordered_set<std::string> contested_dests;
    std::unordered_set<std::string> archive_entries_to_read;

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
            contested_dests.insert(dest);
            for (const auto& src : candidate_sources)
                archive_entries_to_read.insert(src);
        }
    }

    if (contested_dests.empty())
        return;

    logger.log(std::format("[infer] Hashing {} contested dests ({} archive entries)",
                           contested_dests.size(),
                           archive_entries_to_read.size()));

    std::unordered_map<std::string, uint64_t> source_hashes;
    std::unordered_map<std::string, uint64_t> source_sizes;
    std::unordered_set<std::string> missing_entries;

    auto archive_sig = build_archive_signature(archive_path);
    int cache_hits = 0;
    {
        std::scoped_lock lock(cache_mutex);
        for (const auto& entry_path : archive_entries_to_read)
        {
            auto key = std::format("{}|{}", archive_sig, entry_path);
            auto it = archive_entry_hash_cache.find(key);
            if (it != archive_entry_hash_cache.end())
            {
                source_hashes[entry_path] = it->second.hash;
                source_sizes[entry_path] = it->second.size;
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

        for (auto& [entry_path, data] : batch_data)
        {
            uint64_t h = fnv1a_hash(data.data(), data.size());
            uint64_t sz = static_cast<uint64_t>(data.size());
            source_hashes[entry_path] = h;
            source_sizes[entry_path] = sz;

            auto key = std::format("{}|{}", archive_sig, entry_path);
            std::scoped_lock lock(cache_mutex);
            archive_entry_hash_cache[key] = FomodInferenceService::CachedHash{h, sz};
        }
    }
    logger.log(std::format(
        "[infer] Contested hash cache: hits={}, misses={}", cache_hits, missing_entries.size()));

    // Update atoms in the index with hashes
    for (auto& [dest, atom_vec] : atom_index)
    {
        if (!contested_dests.count(dest))
            continue;
        for (auto& a : atom_vec)
        {
            auto sh = source_hashes.find(a.source_path);
            if (sh != source_hashes.end())
            {
                a.content_hash = sh->second;
                auto ss = source_sizes.find(a.source_path);
                if (ss != source_sizes.end())
                    a.file_size = ss->second;
            }
        }
    }

    // Also update atoms in the ExpandedAtoms struct
    atoms.for_each(
        [&](FomodAtom& a)
        {
            auto sh = source_hashes.find(a.source_path);
            if (sh != source_hashes.end())
                a.content_hash = sh->second;
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

        std::ifstream ifs(full_path, std::ios::binary);
        if (!ifs)
            continue;

        // Read entire file and hash
        std::vector<char> buf(static_cast<size_t>(sz));
        ifs.read(buf.data(), static_cast<std::streamsize>(sz));
        auto bytes_read = static_cast<size_t>(ifs.gcount());

        it->second.hash = fnv1a_hash(buf.data(), bytes_read);
        it->second.size = static_cast<uint64_t>(bytes_read);
    }
}

// ---------------------------------------------------------------------------
// Assemble JSON from solver result
// ---------------------------------------------------------------------------

static json assemble_json(const FomodInstaller& installer, const SolverResult& result)
{
    json j_steps = json::array();
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        json j_step;
        j_step["name"] = step.name;
        json j_groups = json::array();

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            const auto& group = step.groups[gi];
            json j_group;
            j_group["name"] = group.name;

            std::vector<std::string> selected;
            std::vector<std::string> deselected;

            for (size_t pi = 0; pi < group.plugins.size(); ++pi)
            {
                bool sel = false;
                if (si < result.selections.size() && gi < result.selections[si].size() &&
                    pi < result.selections[si][gi].size())
                {
                    sel = result.selections[si][gi][pi];
                }

                if (sel)
                    selected.push_back(group.plugins[pi].name);
                else
                    deselected.push_back(group.plugins[pi].name);
            }

            j_group["plugins"] = selected;
            j_group["deselected"] = deselected;
            j_groups.push_back(j_group);
        }
        j_step["groups"] = j_groups;
        j_steps.push_back(j_step);
    }

    json out;
    out["steps"] = j_steps;
    return out;
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
    auto archive_size_mb =
        fs::exists(archive_path)
            ? static_cast<double>(fs::file_size(archive_path)) / (1024.0 * 1024.0)
            : 0.0;

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
        ArchiveService archive_service;

        // Step 1: List archive entries with sizes
        logger.log("[infer] 1/9 Listing archive entries");
        t_step = clock::now();
        auto listing = archive_service.list_entries_with_sizes(archive_path);
        auto& archive_entries = listing.paths;
        auto& entry_sizes = listing.sizes;
        auto t_list = ms_since(t_step);
        logger.log(std::format("[infer] Step 1 list_entries: {} entries, {} sizes ({}ms)",
                               archive_entries.size(),
                               entry_sizes.size(),
                               t_list));

        // Build sorted normalized entry index for O(log N) prefix lookups
        // Also build normalized sizes map (entry_sizes uses original paths)
        std::vector<std::string> sorted_norm_entries;
        std::unordered_map<std::string, uint64_t> norm_entry_sizes;
        sorted_norm_entries.reserve(archive_entries.size());
        for (const auto& entry : archive_entries)
        {
            if (entry.ends_with("/") || entry.ends_with("\\"))
                continue;
            auto norm = normalize_path(entry);
            sorted_norm_entries.push_back(norm);
            auto sz_it = entry_sizes.find(norm);
            if (sz_it != entry_sizes.end())
                norm_entry_sizes[norm] = sz_it->second;
        }
        std::sort(sorted_norm_entries.begin(), sorted_norm_entries.end());

        // Step 2: Find the FOMOD ModuleConfig entry (prefer shallowest path)
        logger.log("[infer] 2/9 Finding FOMOD config");
        std::string fomod_prefix;
        std::string xml_entry_norm;
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
                 (xml_entry_norm.empty() || norm.size() < xml_entry_norm.size())))
            {
                xml_entry_norm = norm;
                best_depth = depth;
            }
        }

        if (xml_entry_norm.empty())
        {
            logger.log(std::format("[infer] Not a FOMOD mod, total: {}ms", ms_since(t_total)));
            return "";
        }

        size_t suffix_pos = xml_entry_norm.size() - module_cfg_suffix.size();
        if (suffix_pos > 0 && xml_entry_norm[suffix_pos - 1] == '/')
            suffix_pos--;
        fomod_prefix = (suffix_pos > 0) ? xml_entry_norm.substr(0, suffix_pos) : "";

        logger.log(std::format(
            "[infer] Step 2 found XML: \"{}\" (prefix: \"{}\")", xml_entry_norm, fomod_prefix));

        // Step 3: Read ModuleConfig.xml into memory
        logger.log("[infer] 3/9 Reading ModuleConfig.xml");
        t_step = clock::now();
        std::unordered_set<std::string> xml_set{xml_entry_norm};
        auto xml_data = archive_service.read_entries_batch(archive_path, xml_set);

        if (xml_data.find(xml_entry_norm) == xml_data.end())
        {
            logger.log_error("[infer] Failed to read ModuleConfig.xml from archive");
            return "";
        }
        const auto& xml_bytes = xml_data[xml_entry_norm];
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

        auto installer = FomodIRParser::parse(doc, fomod_prefix);
        logger.log(std::format("[infer] Step 4 parse IR: {} steps, {} cond patterns ({}ms)",
                               installer.steps.size(),
                               installer.conditional_patterns.size(),
                               ms_since(t_step)));

        // Step 5: Expand atoms
        logger.log("[infer] 5/9 Expanding file atoms");
        t_step = clock::now();
        auto atoms = expand_all_atoms(installer, sorted_norm_entries, norm_entry_sizes);

        int total_atoms = 0;
        atoms.for_each([&](const FomodAtom&) { ++total_atoms; });

        auto atom_index = build_atom_index(atoms);
        auto excluded = compute_excluded_dests(atom_index);
        logger.log(
            std::format("[infer] Step 5 expand atoms: {} total, {} dests, {} excluded ({}ms)",
                        total_atoms,
                        atom_index.size(),
                        excluded.size(),
                        ms_since(t_step)));

        // Step 6: Build target tree
        logger.log("[infer] 6/9 Scanning installed files");
        t_step = clock::now();
        auto installed = scan_installed_files(fs::path(mod_path));
        auto target = build_target_tree(installed, fs::path(mod_path), atom_index, excluded);
        auto t_scan = ms_since(t_step);
        logger.log(
            std::format("[infer] Step 6 target tree: {} files ({}ms)", target.size(), t_scan));

        // Step 7: Hash contested files for disambiguation
        logger.log("[infer] 7/9 Hashing contested files");
        t_step = clock::now();
        hash_contested_files(target,
                             atoms,
                             atom_index,
                             fs::path(mod_path),
                             archive_path,
                             excluded,
                             cache_mutex_,
                             archive_entry_hash_cache_);
        logger.log(std::format("[infer] Step 7 hash contested ({}ms)", ms_since(t_step)));

        // Step 7b: Pre-compute which conditional patterns have target-only files
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
                        if (!has_target_hit &&
                            flat_idx_sv < static_cast<int>(atoms.per_plugin.size()))
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

        // Step 7c: Constraint propagation pre-pass
        t_step = clock::now();
        auto propagation =
            propagate(installer, atoms, atom_index, target, excluded, overrides, nullptr);
        logger.log(std::format(
            "[infer] Step 7c propagate: resolved={}/{} groups, fully_resolved={} ({}ms)",
            propagation.resolved_groups.size(),
            [&]()
            {
                int g = 0;
                for (const auto& s : installer.steps)
                    g += static_cast<int>(s.groups.size());
                return g;
            }(),
            propagation.fully_resolved,
            ms_since(t_step)));

        // Step 8: CSP solve (with propagation-narrowed domains)
        logger.log("[infer] 8/9 Solving (this may take a while)");
        t_step = clock::now();
        auto result = solve_fomod_csp(installer,
                                      atoms,
                                      atom_index,
                                      target,
                                      excluded,
                                      &overrides,
                                      propagation.resolved_groups.empty() ? nullptr : &propagation);
        auto t_solve = ms_since(t_step);
        logger.log(std::format("[infer] Step 8 solve: {} nodes, exact={} ({}ms)",
                               result.nodes_explored,
                               result.exact_match,
                               t_solve));

        // Step 9: Assemble JSON
        logger.log("[infer] 9/9 Assembling result");
        t_step = clock::now();
        auto json_result = assemble_json(installer, result);
        auto result_str = json_result.dump(2);
        logger.log(std::format(
            "[infer] Step 9 assemble JSON: {} bytes ({}ms)", result_str.size(), ms_since(t_step)));

        auto total_ms = ms_since(t_total);
        logger.log(std::format("[infer] DONE total={}ms | list={}ms scan={}ms solve={}ms",
                               total_ms,
                               t_list,
                               t_scan,
                               t_solve));
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

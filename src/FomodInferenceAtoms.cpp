#include "FomodInferenceAtoms.h"

#include <algorithm>
#include <format>

namespace mo2core
{

// ---------------------------------------------------------------------------
// Atom expansion: resolve IR file entries into concrete atoms
// ---------------------------------------------------------------------------

// Delegate to the shared implementation in Utils.h/cpp.
bool is_safe_dest(const std::string& dest)
{
    return is_safe_destination(dest);
}

void expand_entry(const FomodFileEntry& entry,
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
        // An empty source means "root of archive"; use empty prefix so that
        // lower_bound starts at the beginning of the sorted entry list.
        auto prefix = entry.source;
        if (prefix.empty())
        {
            // Match all entries -- leave prefix empty and skip the slash append
        }
        else if (!prefix.ends_with("/"))
        {
            prefix += "/";
        }

        auto it = std::lower_bound(sorted_entries.begin(), sorted_entries.end(), prefix);
        while (it != sorted_entries.end() && it->starts_with(prefix))
        {
            auto rel = it->substr(prefix.size());
            auto dest = entry.destination.empty() ? rel : (entry.destination + "/" + rel);

            auto norm_dest = normalize_path(dest);
            if (!is_safe_dest(norm_dest))
            {
                Logger::instance().log_warning(
                    std::format("[infer] Skipping atom with unsafe destination: {}", norm_dest));
                ++it;
                continue;
            }

            FomodAtom atom;
            atom.source_path = *it;
            atom.dest_path = std::move(norm_dest);
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
        if (!is_safe_dest(entry.destination))
        {
            Logger::instance().log_warning(std::format(
                "[infer] Skipping atom with unsafe destination: {}", entry.destination));
            return;
        }

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

ExpandedAtoms expand_all_atoms(const FomodInstaller& installer,
                               const std::vector<std::string>& sorted_entries,
                               const std::unordered_map<std::string, uint64_t>& entry_sizes)
{
    // Three separate passes are intentional: they encode different document_order
    // ranges to ensure correct priority semantics in the FOMOD spec:
    //   Pass 1: required files (lowest document_order range)
    //   Pass 2: normal plugin files (middle range)
    //   Pass 3: auto-install files (highest range, installed last, wins ties)
    // Merging them into a single loop would break the document_order invariant.
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

AtomIndex build_atom_index(const ExpandedAtoms& atoms)
{
    AtomIndex index;
    atoms.for_each([&](const FomodAtom& a) { index[a.dest_path].push_back(a); });
    return index;
}

std::unordered_set<std::string> compute_excluded_dests(const AtomIndex& atom_index)
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
            else if (a.origin == FomodAtom::Origin::Required)
            {
                // Required files are always installed, but keeping them in the
                // comparison lets the solver detect incomplete installations
                // where expected Required files are missing from the target.
                all_auto = false;
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

TargetTree build_target_tree(const std::unordered_map<std::string, uint64_t>& installed_files,
                             const AtomIndex& atom_index,
                             const std::unordered_set<std::string>& excluded)
{
    TargetTree target;
    for (const auto& [rel_path, file_size] : installed_files)
    {
        // Skip MO2 metadata files that are never part of FOMOD installations
        if (rel_path == "meta.ini")
            continue;

        TargetFile tf;
        tf.size = file_size;
        target[rel_path] = tf;
    }
    return target;
}

// ---------------------------------------------------------------------------
// Assemble JSON from solver result
// ---------------------------------------------------------------------------

nlohmann::json assemble_json(const FomodInstaller& installer, const SolverResult& result)
{
    nlohmann::json j_steps = nlohmann::json::array();
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        nlohmann::json j_step;
        j_step["name"] = step.name;
        nlohmann::json j_groups = nlohmann::json::array();

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            const auto& group = step.groups[gi];
            nlohmann::json j_group;
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
                else
                {
                    Logger::instance().log_warning(
                        std::format("[infer] assemble_json: selection index out of bounds "
                                    "(step={}, group={}, plugin={}), defaulting to false",
                                    si,
                                    gi,
                                    pi));
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

    nlohmann::json out;
    out["steps"] = j_steps;
    return out;
}

}  // namespace mo2core

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

TargetTree build_target_tree(const std::unordered_map<std::string, uint64_t>& installed_files)
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

namespace
{

// Build a plugin object from name + diagnostic fields. The wire format always
// carries `name` and `selected`; `confidence` and `reasons` ride along when
// the diagnostic record exists for this position.
nlohmann::json build_plugin_object(const std::string& name,
                                   bool selected,
                                   const PluginDiagnostics* diag)
{
    nlohmann::json j;
    j["name"] = name;
    j["selected"] = selected;
    if (diag != nullptr)
    {
        j["confidence"] = serialize_confidence(diag->confidence);
        nlohmann::json reasons = nlohmann::json::array();
        for (const auto& r : diag->reasons)
        {
            reasons.push_back(serialize_reason(r));
        }
        j["reasons"] = std::move(reasons);
    }
    return j;
}

const PluginDiagnostics* lookup_plugin_diag(const InferenceDiagnostics& diag,
                                            size_t s,
                                            size_t g,
                                            size_t p)
{
    if (s >= diag.steps.size())
    {
        return nullptr;
    }
    const auto& step = diag.steps[s];
    if (g >= step.groups.size())
    {
        return nullptr;
    }
    const auto& group = step.groups[g];
    if (p >= group.plugins.size())
    {
        return nullptr;
    }
    return &group.plugins[p];
}

const GroupDiagnostics* lookup_group_diag(const InferenceDiagnostics& diag, size_t s, size_t g)
{
    if (s >= diag.steps.size())
    {
        return nullptr;
    }
    const auto& step = diag.steps[s];
    if (g >= step.groups.size())
    {
        return nullptr;
    }
    return &step.groups[g];
}

const StepDiagnostics* lookup_step_diag(const InferenceDiagnostics& diag, size_t s)
{
    if (s >= diag.steps.size())
    {
        return nullptr;
    }
    return &diag.steps[s];
}

}  // namespace

nlohmann::json assemble_json(const FomodInstaller& installer,
                             const SolverResult& result,
                             const InferenceDiagnostics& diagnostics)
{
    nlohmann::json j_steps = nlohmann::json::array();
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        nlohmann::json j_step;
        j_step["name"] = step.name;

        const StepDiagnostics* step_diag = lookup_step_diag(diagnostics, si);
        if (step_diag != nullptr)
        {
            j_step["confidence"] = serialize_confidence(step_diag->confidence);
            j_step["visible"] = step_diag->visible;
            nlohmann::json reasons = nlohmann::json::array();
            for (const auto& r : step_diag->reasons)
            {
                reasons.push_back(serialize_reason(r));
            }
            j_step["reasons"] = std::move(reasons);
        }

        nlohmann::json j_groups = nlohmann::json::array();
        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            const auto& group = step.groups[gi];
            nlohmann::json j_group;
            j_group["name"] = group.name;

            const GroupDiagnostics* group_diag = lookup_group_diag(diagnostics, si, gi);
            if (group_diag != nullptr)
            {
                j_group["confidence"] = serialize_confidence(group_diag->confidence);
                j_group["resolved_by"] = group_diag->resolved_by;
                nlohmann::json reasons = nlohmann::json::array();
                for (const auto& r : group_diag->reasons)
                {
                    reasons.push_back(serialize_reason(r));
                }
                j_group["reasons"] = std::move(reasons);
            }

            nlohmann::json j_selected = nlohmann::json::array();
            nlohmann::json j_deselected = nlohmann::json::array();
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

                const PluginDiagnostics* plugin_diag = lookup_plugin_diag(diagnostics, si, gi, pi);
                auto j_plugin = build_plugin_object(group.plugins[pi].name, sel, plugin_diag);
                if (sel)
                {
                    j_selected.push_back(std::move(j_plugin));
                }
                else
                {
                    j_deselected.push_back(std::move(j_plugin));
                }
            }
            j_group["plugins"] = std::move(j_selected);
            j_group["deselected"] = std::move(j_deselected);
            j_groups.push_back(std::move(j_group));
        }
        j_step["groups"] = std::move(j_groups);
        j_steps.push_back(std::move(j_step));
    }

    nlohmann::json out;
    out["schema_version"] = diagnostics.schema_version;
    out["steps"] = std::move(j_steps);
    out["diagnostics"] = serialize_run_diagnostics(diagnostics.run);
    return out;
}

}  // namespace mo2core

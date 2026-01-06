#include "FomodPropagator.h"
#include "FomodDependencyEvaluator.h"
#include "Logger.h"

#include <algorithm>
#include <cassert>
#include <format>
#include <unordered_set>

namespace mo2core
{

PropagationResult propagate(const FomodInstaller& installer,
                            const ExpandedAtoms& atoms,
                            const AtomIndex& atom_index,
                            const TargetTree& target,
                            const std::unordered_set<std::string>& excluded_dests,
                            const InferenceOverrides& overrides,
                            const FomodDependencyContext* context)
{
    auto& logger = Logger::instance();
    PropagationResult result;

    // Initialize narrowed_domains: [step][group][plugin] = true
    result.narrowed_domains.resize(installer.steps.size());
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        result.narrowed_domains[si].resize(installer.steps[si].groups.size());
        for (size_t gi = 0; gi < installer.steps[si].groups.size(); ++gi)
            result.narrowed_domains[si][gi].assign(installer.steps[si].groups[gi].plugins.size(),
                                                   true);
    }

    // Track which groups are resolved (only mark once).
    // resolved[si][gi] = true means the group has been deterministically resolved.
    std::vector<std::vector<bool>> resolved(installer.steps.size());
    for (size_t si = 0; si < installer.steps.size(); ++si)
        resolved[si].assign(installer.steps[si].groups.size(), false);

    // Build flat_start index: maps (step_idx, group_idx) -> flat plugin start index.
    auto flat_starts = compute_flat_starts(installer);

    std::unordered_map<std::string, std::string> flags;

    // Fixpoint iteration: repeat until no domain changes occur.
    //
    // Why fixpoint iteration instead of topological ordering or single-pass?
    // FOMOD flag dependencies can form cycles (group A sets a flag that group B
    // reads, and B sets a flag that A reads). A single forward pass misses
    // backward propagation. Topological ordering requires acyclicity. Fixpoint
    // iteration handles arbitrary dependency shapes correctly.
    //
    // Cap at 16 iterations: empirically, 2-3 iterations suffice for all known
    // FOMOD installers (flag chains are typically short). 16 provides a wide
    // safety margin for pathological circular dependencies without risking
    // unbounded loops. The theoretical upper bound is N (number of groups) but
    // that is unreachable in practice.
    static constexpr int MAX_ITERATIONS = 16;
    int total_resolved = 0;

    for (int iteration = 0; iteration < MAX_ITERATIONS; ++iteration)
    {
        bool changed = false;

        for (size_t si = 0; si < installer.steps.size(); ++si)
        {
            const auto& step = installer.steps[si];

            // All steps treated as visible -- we cannot determine original visibility.

            for (size_t gi = 0; gi < step.groups.size(); ++gi)
            {
                if (resolved[si][gi])
                    continue;

                const auto& group = step.groups[gi];
                auto& domain = result.narrowed_domains[si][gi];
                int n = static_cast<int>(group.plugins.size());
                int flat_start = flat_starts[si][gi];

                // 1. Evaluate plugin types, eliminate NotUsable.
                for (int pi = 0; pi < n; ++pi)
                {
                    if (!domain[pi])
                        continue;
                    const auto& plugin = group.plugins[pi];
                    auto eff = evaluate_plugin_type(plugin, flags, context);

                    // During inference (no external context), dynamic dependencyType
                    // outcomes are not definitive enough for hard domain pruning.
                    bool dynamic_without_context = (!context && !plugin.type_patterns.empty());
                    if (eff == PluginType::NotUsable)
                    {
                        if (!dynamic_without_context)
                        {
                            domain[pi] = false;
                            changed = true;
                        }
                    }
                }

                // 2. Filter by file evidence.
                // For each usable plugin, collect its non-auto-install atoms that are unique
                // within this group (no other usable plugin in this group produces the same dest).
                // If ALL such unique atoms miss the target, eliminate the plugin.
                {
                    // Collect per-plugin dest sets (non-auto atoms only).
                    std::vector<std::unordered_set<std::string>> plugin_dests(n);
                    for (int pi = 0; pi < n; ++pi)
                    {
                        if (!domain[pi])
                            continue;
                        int fi = flat_start + pi;
                        if (fi < static_cast<int>(atoms.per_plugin.size()))
                        {
                            for (const auto& atom : atoms.per_plugin[fi])
                            {
                                if (!atom.always_install && !atom.install_if_usable &&
                                    !excluded_dests.count(atom.dest_path))
                                    plugin_dests[pi].insert(atom.dest_path);
                            }
                        }
                    }

                    // Build map: dest -> usable plugin indices within this group.
                    std::unordered_map<std::string, std::vector<int>> dest_to_plugins_local;
                    for (int pi = 0; pi < n; ++pi)
                    {
                        if (!domain[pi])
                            continue;
                        for (const auto& d : plugin_dests[pi])
                            dest_to_plugins_local[d].push_back(pi);
                    }

                    for (int pi = 0; pi < n; ++pi)
                    {
                        if (!domain[pi] || plugin_dests[pi].empty())
                            continue;

                        // Find dests unique to this plugin within the group.
                        bool has_any_unique = false;
                        bool all_unique_miss = true;
                        for (const auto& d : plugin_dests[pi])
                        {
                            auto it = dest_to_plugins_local.find(d);
                            if (it != dest_to_plugins_local.end() && it->second.size() == 1)
                            {
                                has_any_unique = true;
                                if (target.count(d))
                                {
                                    all_unique_miss = false;
                                    break;
                                }
                            }
                        }

                        if (has_any_unique && all_unique_miss)
                        {
                            domain[pi] = false;
                            changed = true;
                        }
                    }
                }

                // 3. Enforce cardinality constraints.
                int usable_count = static_cast<int>(std::count(domain.begin(), domain.end(), true));

                bool group_resolved = false;
                switch (group.type)
                {
                    case FomodGroupType::SelectAll:
                        // All usable plugins are selected.
                        group_resolved = true;
                        break;
                    case FomodGroupType::SelectExactlyOne:
                        if (usable_count == 1)
                            group_resolved = true;
                        break;
                    case FomodGroupType::SelectAtMostOne:
                        // Only resolve when forced to zero. When usable_count == 1,
                        // "select zero" is still valid, so leave it for the CSP solver.
                        if (usable_count == 0)
                            group_resolved = true;
                        break;
                    case FomodGroupType::SelectAtLeastOne:
                        if (usable_count == 1)
                            group_resolved = true;
                        break;
                    case FomodGroupType::SelectAny:
                        if (usable_count == 0)
                            group_resolved = true;
                        break;
                }

                if (group_resolved)
                {
                    resolved[si][gi] = true;
                    result.resolved_groups.emplace_back(static_cast<int>(si), static_cast<int>(gi));
                    total_resolved++;
                    changed = true;

                    // Accumulate flags from selected plugins in a resolved group.
                    // When group_resolved is true, domain[pi]==true means the plugin
                    // is definitively selected (the cardinality constraint has been
                    // fully determined).
                    assert(group_resolved &&
                           "Flags should only be accumulated for resolved groups");
                    for (int pi = 0; pi < n; ++pi)
                    {
                        if (domain[pi])
                        {
                            for (const auto& [name, value] : group.plugins[pi].condition_flags)
                                flags[name] = value;
                        }
                    }
                }
            }
        }

        if (!changed)
            break;
    }

    // Check if all groups are resolved.
    int total_groups = 0;
    for (size_t si = 0; si < installer.steps.size(); ++si)
        total_groups += static_cast<int>(installer.steps[si].groups.size());

    result.fully_resolved = (total_resolved == total_groups);

    logger.log(std::format("[propagate] resolved {}/{} groups, fully_resolved={}",
                           total_resolved,
                           total_groups,
                           result.fully_resolved));

    return result;
}

}  // namespace mo2core

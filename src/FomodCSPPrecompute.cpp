#include "FomodCSPPrecompute.h"

#include "FomodDependencyEvaluator.h"

#include <algorithm>
#include <queue>

namespace mo2core
{

void collect_condition_flags(const FomodCondition& c, std::unordered_set<std::string>& out)
{
    if (c.type == FomodConditionType::Flag)
    {
        if (!c.flag_name.empty())
            out.insert(c.flag_name);
        return;
    }
    if (c.type == FomodConditionType::Composite)
    {
        for (const auto& child : c.children)
            collect_condition_flags(child, out);
    }
}

bool condition_depends_on_external_state(const FomodCondition& c)
{
    switch (c.type)
    {
        case FomodConditionType::Flag:
            return false;
        case FomodConditionType::Composite:
            for (const auto& child : c.children)
                if (condition_depends_on_external_state(child))
                    return true;
            return false;
        case FomodConditionType::File:
        case FomodConditionType::Game:
        case FomodConditionType::Plugin:
        case FomodConditionType::Fomod:
        case FomodConditionType::Fomm:
        case FomodConditionType::Fose:
            return true;
    }
    return true;
}

void hash_combine(uint64_t& seed, uint64_t v)
{
    seed ^= v + 0x9e3779b97f4a7c15ULL + (seed << 6U) + (seed >> 2U);
}

uint64_t hash_flag_subset(const std::unordered_map<std::string, std::string>& flags,
                          const std::vector<std::string>& keys)
{
    uint64_t h = 14695981039346656037ULL;
    for (const auto& k : keys)
    {
        hash_combine(h, fnv1a_hash(k.data(), k.size()));
        auto it = flags.find(k);
        if (it != flags.end())
            hash_combine(h, fnv1a_hash(it->second.data(), it->second.size()));
        else
            hash_combine(h, 0xA5A5A5A5A5A5A5A5ULL);
    }
    return h;
}

static void sort_unique(std::vector<int>& v)
{
    std::sort(v.begin(), v.end());
    v.erase(std::unique(v.begin(), v.end()), v.end());
}

using FlagSetterMap = std::unordered_map<std::string, std::vector<std::pair<int, std::string>>>;

// Propagate evidence from a flag-gated condition back to the plugins that set
// the required flag values. This creates an indirect evidence link: if a
// conditional file pattern produces target-matching files, the plugins whose
// flag outputs would activate that pattern gain evidence proportional to the
// number of matching files (the `weight` parameter).
static void collect_flag_evidence(const FomodCondition& cond,
                                  const FlagSetterMap& flag_setters,
                                  std::vector<int>& evidence,
                                  int weight)
{
    if (cond.type == FomodConditionType::Flag)
    {
        auto it = flag_setters.find(cond.flag_name);
        if (it != flag_setters.end())
        {
            for (const auto& [pidx, pval] : it->second)
                if (pval == cond.flag_value)
                    evidence[pidx] += weight;
        }
    }
    else if (cond.type == FomodConditionType::Composite)
    {
        for (const auto& child : cond.children)
            collect_flag_evidence(child, flag_setters, evidence, weight);
    }
}

// Compute per-plugin evidence scores: how strongly each plugin's file output
// correlates with the target file tree. Higher evidence => more likely the
// plugin was selected in the original installation.
//
// Evidence weights for direct file matches:
//   3 = unique producer: only one plugin can produce this target dest, so it
//       is near-certain that plugin was selected. Strong signal.
//   2 = contested dest with hash match: multiple plugins produce this dest,
//       but this one has a matching content hash. Good but not conclusive
//       (another plugin might also hash-match).
//   1 = contested dest, size-compatible but no hash confirmation. Weak signal
//       since any size-compatible producer could be the source.
//
// Indirect evidence via flags: if conditional file patterns match the target,
// the plugins that set the activating flags get evidence weighted by the
// number of matching conditional files.
std::vector<int> compute_evidence(const FomodInstaller& installer,
                                  const ExpandedAtoms& atoms,
                                  const AtomIndex& atom_index,
                                  const TargetTree& target,
                                  const std::unordered_set<std::string>& excluded)
{
    int total_plugins = 0;
    for (const auto& step : installer.steps)
        for (const auto& group : step.groups)
            total_plugins += static_cast<int>(group.plugins.size());

    std::vector<int> evidence(total_plugins, 0);

    for (const auto& [dest, tf] : target)
    {
        if (excluded.count(dest))
            continue;

        auto it = atom_index.find(dest);
        if (it == atom_index.end())
            continue;

        std::vector<int> matches;
        for (const auto& atom : it->second)
        {
            if (atom.origin != FomodAtom::Origin::Plugin)
                continue;
            if (atom.always_install || atom.install_if_usable)
                continue;
            if (tf.size != 0 && atom.file_size != 0 && tf.size != atom.file_size)
                continue;
            matches.push_back(atom.plugin_index);
        }

        if (matches.size() == 1)
        {
            evidence[matches[0]] += 3;  // unique producer: strong signal
        }
        else
        {
            for (int idx : matches)
            {
                bool hash_matched = false;  // contested dest: 2 if hash confirms, 1 otherwise
                for (const auto& atom : it->second)
                {
                    if (atom.plugin_index == idx && tf.hash != 0 && atom.content_hash != 0 &&
                        atom.content_hash == tf.hash)
                    {
                        hash_matched = true;
                        break;
                    }
                }
                evidence[idx] += hash_matched ? 2 : 1;
            }
        }
    }

    FlagSetterMap setters;
    {
        int p = 0;
        for (const auto& step : installer.steps)
        {
            for (const auto& group : step.groups)
            {
                for (const auto& plugin : group.plugins)
                {
                    for (const auto& [fn, fv] : plugin.condition_flags)
                        setters[fn].emplace_back(p, fv);
                    ++p;
                }
            }
        }
    }

    if (!installer.conditional_patterns.empty())
    {
        for (size_t ci = 0; ci < installer.conditional_patterns.size(); ++ci)
        {
            int hits = 0;
            if (ci < atoms.per_conditional.size())
            {
                for (const auto& atom : atoms.per_conditional[ci])
                {
                    if (excluded.count(atom.dest_path))
                        continue;

                    auto tf_it = target.find(atom.dest_path);
                    if (tf_it == target.end())
                        continue;

                    bool size_match = (tf_it->second.size == 0 || atom.file_size == 0 ||
                                       tf_it->second.size == atom.file_size);
                    if (!size_match)
                        continue;

                    bool hash_match = (tf_it->second.hash == 0 || atom.content_hash == 0 ||
                                       tf_it->second.hash == atom.content_hash);
                    if (!hash_match)
                        continue;

                    hits++;
                }
            }
            if (hits > 0)
                collect_flag_evidence(
                    installer.conditional_patterns[ci].condition, setters, evidence, hits);
        }
    }

    // Step-visibility evidence: if a step is conditionally visible and its plugins
    // produce files matching the target, propagate evidence to the flag-setters
    // that control that step's visibility condition.
    {
        int flat_idx = 0;
        for (size_t si = 0; si < installer.steps.size(); ++si)
        {
            const auto& step = installer.steps[si];
            int step_flat_start = flat_idx;
            for (const auto& group : step.groups)
                flat_idx += static_cast<int>(group.plugins.size());

            if (!step.visible)
                continue;
            if (condition_depends_on_external_state(*step.visible))
                continue;

            int hits = 0;
            for (int fi = step_flat_start; fi < flat_idx; ++fi)
            {
                if (fi >= static_cast<int>(atoms.per_plugin.size()))
                    break;
                for (const auto& atom : atoms.per_plugin[fi])
                {
                    if (atom.always_install || atom.install_if_usable)
                        continue;
                    if (excluded.count(atom.dest_path))
                        continue;
                    auto tf_it = target.find(atom.dest_path);
                    if (tf_it == target.end())
                        continue;
                    bool size_match = (tf_it->second.size == 0 || atom.file_size == 0 ||
                                       tf_it->second.size == atom.file_size);
                    if (!size_match)
                        continue;
                    hits++;
                }
            }
            if (hits > 0)
                collect_flag_evidence(*step.visible, setters, evidence, hits);
        }
    }

    return evidence;
}

// Add an undirected edge between two groups in the dependency graph.
// Groups are linked when they share a dest path or participate in the same
// flag setter/reader chain, meaning a change in one can affect the other.
static void link_groups(std::vector<std::unordered_set<int>>& graph, int a, int b)
{
    if (a == b)
        return;
    graph[a].insert(b);
    graph[b].insert(a);
}

// Discover connected components among groups via BFS on an undirected graph.
//
// Graph construction:
//   - Edge between groups that share a destination path (selecting either can
//     change which source file wins at that dest).
//   - Edge between a flag-setter group and any flag-reader group for the same
//     flag (the setter's choice affects the reader's visibility/type outcomes).
//   - Edge between groups that set the same flag (they compete for that flag's
//     value, so their choices are interdependent).
//
// Each connected component can be solved independently, which dramatically
// reduces the effective search space (product of component sizes vs. one
// monolithic exponential). Components are returned largest-first so the solver
// tackles the hardest subproblem while the budget is freshest.
static std::vector<std::vector<int>> build_components(const Precompute& pre)
{
    int n = static_cast<int>(pre.groups.size());
    std::vector<std::unordered_set<int>> graph(static_cast<size_t>(n));

    for (const auto& [dest, groups] : pre.dest_to_groups)
        for (size_t i = 0; i < groups.size(); ++i)
            for (size_t j = i + 1; j < groups.size(); ++j)
                link_groups(graph, groups[i], groups[j]);

    std::unordered_map<std::string, std::vector<int>> setters;
    std::unordered_map<std::string, std::vector<int>> readers;
    for (int g = 0; g < n; ++g)
    {
        for (const auto& fn : pre.group_sets_flags[g])
            setters[fn].push_back(g);
        for (const auto& fn : pre.group_reads_flags[g])
            readers[fn].push_back(g);
    }

    for (const auto& [flag, setv] : setters)
    {
        auto it = readers.find(flag);
        if (it != readers.end())
            for (int s : setv)
                for (int r : it->second)
                    link_groups(graph, s, r);

        for (size_t i = 0; i < setv.size(); ++i)
            for (size_t j = i + 1; j < setv.size(); ++j)
                link_groups(graph, setv[i], setv[j]);
    }

    std::vector<std::vector<int>> comps;
    std::vector<bool> seen(static_cast<size_t>(n), false);
    for (int i = 0; i < n; ++i)
    {
        if (seen[static_cast<size_t>(i)])
            continue;

        std::vector<int> comp;
        std::queue<int> q;
        q.push(i);
        seen[static_cast<size_t>(i)] = true;

        while (!q.empty())
        {
            int cur = q.front();
            q.pop();
            comp.push_back(cur);

            for (int nxt : graph[cur])
            {
                if (seen[static_cast<size_t>(nxt)])
                    continue;
                seen[static_cast<size_t>(nxt)] = true;
                q.push(nxt);
            }
        }

        std::sort(comp.begin(), comp.end());
        comps.push_back(std::move(comp));
    }

    std::sort(comps.begin(),
              comps.end(),
              [](const auto& a, const auto& b) { return a.size() > b.size(); });
    return comps;
}

Precompute build_precompute(const FomodInstaller& installer,
                            const ExpandedAtoms& atoms,
                            const AtomIndex& atom_index,
                            const TargetTree& target,
                            const std::unordered_set<std::string>& excluded,
                            const InferenceOverrides* overrides,
                            const PropagationResult* propagation,
                            std::vector<GroupRef> groups,
                            std::vector<int> evidence)
{
    Precompute p;
    p.installer = &installer;
    p.atoms = &atoms;
    p.atom_index = &atom_index;
    p.target = &target;
    p.excluded = &excluded;
    p.overrides = overrides;
    p.propagation = propagation;
    p.groups = std::move(groups);
    p.evidence = std::move(evidence);

    int total_plugins = 0;
    for (const auto& step : installer.steps)
        for (const auto& group : step.groups)
            total_plugins += static_cast<int>(group.plugins.size());

    p.plugin_to_group.assign(static_cast<size_t>(total_plugins), -1);
    p.plugin_unique_support.assign(static_cast<size_t>(total_plugins), 0);

    p.group_sets_flags.resize(p.groups.size());
    p.group_reads_flags.resize(p.groups.size());
    p.group_cache_flags.resize(p.groups.size());
    p.group_dests.resize(p.groups.size());

    for (const auto& step : installer.steps)
    {
        if (step.visible)
            collect_condition_flags(*step.visible, p.needed_flags);

        for (const auto& group : step.groups)
        {
            for (const auto& plugin : group.plugins)
            {
                for (const auto& tp : plugin.type_patterns)
                    collect_condition_flags(tp.condition, p.needed_flags);
                if (plugin.dependencies)
                    collect_condition_flags(*plugin.dependencies, p.needed_flags);
            }
        }
    }

    for (const auto& cp : installer.conditional_patterns)
        collect_condition_flags(cp.condition, p.needed_flags);

    for (int gidx = 0; gidx < static_cast<int>(p.groups.size()); ++gidx)
    {
        const auto& gref = p.groups[gidx];
        const auto& step = installer.steps[gref.step_idx];
        const auto& group = step.groups[gref.group_idx];

        if (step.visible)
            collect_condition_flags(*step.visible, p.group_reads_flags[gidx]);

        for (int pi = 0; pi < gref.plugin_count; ++pi)
        {
            int flat_plugin = gref.flat_start + pi;
            p.plugin_to_group[flat_plugin] = gidx;
            const auto& plugin = group.plugins[static_cast<size_t>(pi)];

            for (const auto& [fn, fv] : plugin.condition_flags)
                p.group_sets_flags[gidx].insert(fn);
            for (const auto& tp : plugin.type_patterns)
                collect_condition_flags(tp.condition, p.group_reads_flags[gidx]);
            if (plugin.dependencies)
                collect_condition_flags(*plugin.dependencies, p.group_reads_flags[gidx]);

            if (flat_plugin < static_cast<int>(atoms.per_plugin.size()))
            {
                std::unordered_set<std::string> seen;
                for (const auto& atom : atoms.per_plugin[flat_plugin])
                {
                    if (excluded.count(atom.dest_path))
                        continue;
                    if (!seen.insert(atom.dest_path).second)
                        continue;

                    p.group_dests[gidx].insert(atom.dest_path);
                    p.dest_to_groups[atom.dest_path].push_back(gidx);
                }
            }
        }
    }

    for (auto& [dest, groups_vec] : p.dest_to_groups)
        sort_unique(groups_vec);

    for (int gidx = 0; gidx < static_cast<int>(p.group_sets_flags.size()); ++gidx)
    {
        for (const auto& fn : p.group_sets_flags[gidx])
            p.flag_to_setter_groups[fn].push_back(gidx);
    }
    for (auto& [fn, setters] : p.flag_to_setter_groups)
        sort_unique(setters);

    for (size_t g = 0; g < p.group_cache_flags.size(); ++g)
    {
        std::vector<std::string> keys(p.group_reads_flags[g].begin(), p.group_reads_flags[g].end());
        std::sort(keys.begin(), keys.end());
        p.group_cache_flags[g] = std::move(keys);
    }

    {
        std::unordered_set<std::string> memo_flag_set = p.needed_flags;
        for (const auto& s : p.group_sets_flags)
            for (const auto& fn : s)
                memo_flag_set.insert(fn);
        p.memo_flags.assign(memo_flag_set.begin(), memo_flag_set.end());
        std::sort(p.memo_flags.begin(), p.memo_flags.end());
    }

    std::unordered_set<int> contested_plugins;

    for (const auto& [dest, tf] : target)
    {
        if (excluded.count(dest))
            continue;

        auto it = atom_index.find(dest);
        if (it == atom_index.end())
            continue;

        std::vector<int> size_match_groups;
        std::vector<int> hash_capable_groups;
        std::unordered_set<int> candidate_plugins;
        std::unordered_set<int> producer_plugins;
        std::unordered_set<std::string> sources;
        bool has_conditional = false;

        for (const auto& atom : it->second)
        {
            if (atom.origin == FomodAtom::Origin::Conditional)
                has_conditional = true;

            if (atom.origin != FomodAtom::Origin::Plugin || atom.plugin_index < 0)
                continue;

            producer_plugins.insert(atom.plugin_index);

            if (atom.plugin_index >= static_cast<int>(p.plugin_to_group.size()))
                continue;

            int gidx = p.plugin_to_group[atom.plugin_index];
            if (gidx < 0)
                continue;

            bool size_match = (tf.size == 0 || atom.file_size == 0 || tf.size == atom.file_size);
            bool hash_capable = size_match && (tf.hash == 0 || atom.content_hash == 0 ||
                                               tf.hash == atom.content_hash);

            if (size_match)
                size_match_groups.push_back(gidx);
            if (hash_capable)
                hash_capable_groups.push_back(gidx);

            bool candidate = size_match;
            if (candidate && tf.hash != 0 && atom.content_hash != 0 && tf.hash != atom.content_hash)
                candidate = false;
            if (candidate)
                candidate_plugins.insert(atom.plugin_index);

            sources.insert(atom.source_path);
            contested_plugins.insert(atom.plugin_index);
        }

        sort_unique(size_match_groups);
        sort_unique(hash_capable_groups);

        std::vector<int> producers_vec(producer_plugins.begin(), producer_plugins.end());
        sort_unique(producers_vec);
        p.dest_to_plugins[dest] = std::move(producers_vec);

        p.dest_to_size_match_groups[dest] = std::move(size_match_groups);
        p.dest_to_hash_capable_groups[dest] = std::move(hash_capable_groups);

        if (candidate_plugins.size() == 1)
        {
            int unique_plugin = *candidate_plugins.begin();
            if (unique_plugin >= 0 &&
                unique_plugin < static_cast<int>(p.plugin_unique_support.size()))
                p.plugin_unique_support[unique_plugin]++;
        }

        if (has_conditional)
            p.conditional_dests.insert(dest);

        if (sources.size() > 1)
            for (int flat_plugin : candidate_plugins)
                contested_plugins.insert(flat_plugin);
    }

    p.contested_plugins.assign(contested_plugins.begin(), contested_plugins.end());
    std::sort(p.contested_plugins.begin(), p.contested_plugins.end());

    p.components = build_components(p);
    return p;
}

}  // namespace mo2core

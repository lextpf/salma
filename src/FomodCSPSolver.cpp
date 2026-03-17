#include "FomodCSPSolver.h"
#include "FomodDependencyEvaluator.h"
#include "FomodForwardSimulator.h"
#include "FomodPropagator.h"
#include "Logger.h"

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <format>
#include <numeric>
#include <queue>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

struct ReproMetrics
{
    int missing = 0;
    int extra = 0;
    int size_mismatch = 0;
    int hash_mismatch = 0;
    int reproduced = 0;

    bool exact() const
    {
        return missing == 0 && extra == 0 && size_mismatch == 0 && hash_mismatch == 0;
    }

    bool better_than(const ReproMetrics& rhs) const
    {
        if (missing != rhs.missing)
            return missing < rhs.missing;
        if (extra != rhs.extra)
            return extra < rhs.extra;
        if (size_mismatch != rhs.size_mismatch)
            return size_mismatch < rhs.size_mismatch;
        if (hash_mismatch != rhs.hash_mismatch)
            return hash_mismatch < rhs.hash_mismatch;
        if (reproduced != rhs.reproduced)
            return reproduced > rhs.reproduced;
        return false;
    }
};

struct GroupRef
{
    int step_idx = 0;
    int group_idx = 0;
    int flat_start = 0;
    int plugin_count = 0;
};

using GroupOption = std::vector<bool>;

struct SolverState
{
    std::vector<std::vector<std::vector<bool>>> selections;
    std::unordered_map<std::string, std::string> flags;
    int nodes_explored = 0;
    bool found_exact = false;

    SolverResult best;
    ReproMetrics best_metrics;
    bool has_best = false;

    // Progress tracking: log every N nodes
    int last_progress_nodes = 0;
    std::chrono::steady_clock::time_point last_progress_time = std::chrono::steady_clock::now();
    static constexpr int PROGRESS_NODE_INTERVAL = 1'000;
    static constexpr int PROGRESS_TIME_INTERVAL_MS = 1'000;

    int64_t estimated_total = 0;
    std::chrono::steady_clock::time_point pass_start_time = std::chrono::steady_clock::now();
    int64_t pass_start_nodes = 0;

    // Wall-clock deadline for the entire solve (0 = no limit)
    std::chrono::steady_clock::time_point deadline{};
    bool deadline_exceeded = false;
};

struct SolverStats
{
    int dropped_extra_only_options = 0;
    int collapsed_equivalent_options = 0;
    int forced_unique_options = 0;
    int capped_select_any_options = 0;

    int pruned_extra_only = 0;
    int pruned_lower_bound = 0;
    int pruned_memo = 0;
    int skipped_invisible = 0;
    int pruned_node_limit = 0;

    std::vector<bool> logged_group_options;
};

struct OptionProfile
{
    GroupOption option;
    int evidence_score = 0;
    int unique_support = 0;
    int useful_dests = 0;
    int extra_dests = 0;
    bool sets_needed_flag = false;
    std::unordered_set<std::string> produced;
    std::unordered_set<std::string> produced_atoms;
    std::unordered_map<std::string, std::string> flags_written;
};

struct CachedOptions
{
    std::vector<GroupOption> options;
    std::vector<OptionProfile> profiles;
};

struct Precompute
{
    const FomodInstaller* installer = nullptr;
    const ExpandedAtoms* atoms = nullptr;
    const AtomIndex* atom_index = nullptr;
    const TargetTree* target = nullptr;
    const std::unordered_set<std::string>* excluded = nullptr;
    const InferenceOverrides* overrides = nullptr;
    const PropagationResult* propagation = nullptr;

    std::vector<GroupRef> groups;
    std::vector<int> evidence;

    std::vector<int> plugin_to_group;
    std::vector<int> plugin_unique_support;

    std::unordered_set<std::string> needed_flags;
    std::vector<std::unordered_set<std::string>> group_sets_flags;
    std::vector<std::unordered_set<std::string>> group_reads_flags;
    std::vector<std::vector<std::string>> group_cache_flags;
    std::unordered_map<std::string, std::vector<int>> flag_to_setter_groups;
    std::vector<std::string> memo_flags;
    std::vector<std::unordered_set<std::string>> group_dests;

    std::unordered_map<std::string, std::vector<int>> dest_to_groups;
    std::unordered_map<std::string, std::vector<int>> dest_to_plugins;
    std::unordered_map<std::string, std::vector<int>> dest_to_size_match_groups;
    std::unordered_map<std::string, std::vector<int>> dest_to_hash_capable_groups;

    std::unordered_set<std::string> conditional_dests;
    std::vector<int> contested_plugins;

    std::vector<std::vector<int>> components;
};

struct OptionCacheKey
{
    int group_idx = 0;
    uint64_t flags_sig = 0;
    int select_any_cap = 0;
    bool exact_mode = false;

    bool operator==(const OptionCacheKey& rhs) const
    {
        return group_idx == rhs.group_idx && flags_sig == rhs.flags_sig &&
               select_any_cap == rhs.select_any_cap && exact_mode == rhs.exact_mode;
    }
};

struct OptionCacheKeyHash
{
    size_t operator()(const OptionCacheKey& k) const
    {
        uint64_t h = static_cast<uint64_t>(k.group_idx) * 11400714819323198485ULL;
        h ^= k.flags_sig + 0x9e3779b97f4a7c15ULL + (h << 6U) + (h >> 2U);
        h ^= static_cast<uint64_t>(k.select_any_cap + 1) + 0x9e3779b97f4a7c15ULL + (h << 6U) +
             (h >> 2U);
        h ^= static_cast<uint64_t>(k.exact_mode ? 1 : 0) + 0x9e3779b97f4a7c15ULL + (h << 6U) +
             (h >> 2U);
        return static_cast<size_t>(h);
    }
};

struct MemoKey
{
    int next_idx = 0;
    uint64_t flag_state_sig = 0;
    uint64_t contested_sig = 0;

    bool operator==(const MemoKey& rhs) const
    {
        return next_idx == rhs.next_idx && flag_state_sig == rhs.flag_state_sig &&
               contested_sig == rhs.contested_sig;
    }
};

struct MemoKeyHash
{
    size_t operator()(const MemoKey& k) const
    {
        uint64_t h = static_cast<uint64_t>(k.next_idx) * 1099511628211ULL;
        h ^= k.flag_state_sig + 0x9e3779b97f4a7c15ULL + (h << 6U) + (h >> 2U);
        h ^= k.contested_sig + 0x9e3779b97f4a7c15ULL + (h << 6U) + (h >> 2U);
        return static_cast<size_t>(h);
    }
};

struct FlagDelta
{
    std::string name;
    bool had_value = false;
    std::string old_value;
};

struct SearchPlan
{
    std::vector<int> order;
    std::vector<int> order_pos;
    int node_limit = 0;
    std::unordered_map<MemoKey, ReproMetrics, MemoKeyHash> memo;
    bool incremental_flags = false;
};

static constexpr int kSelectAnyCapNarrow = 64;
static constexpr int kSelectAnyCapMedium = 256;
static constexpr int kSelectAnyCapFull = 0;

static bool is_exact_group_mode(int gidx, const std::unordered_set<int>* exact_groups)
{
    return exact_groups && exact_groups->count(gidx) > 0;
}

static int effective_select_any_cap(int gidx,
                                    int select_any_cap,
                                    const std::unordered_set<int>* exact_groups)
{
    if (is_exact_group_mode(gidx, exact_groups))
        return kSelectAnyCapFull;
    return select_any_cap;
}

static void hash_combine(uint64_t& seed, uint64_t v)
{
    seed ^= v + 0x9e3779b97f4a7c15ULL + (seed << 6U) + (seed >> 2U);
}

static void collect_condition_flags(const FomodCondition& c, std::unordered_set<std::string>& out)
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

static bool condition_depends_on_external_state(const FomodCondition& c)
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

static uint64_t hash_flag_subset(const std::unordered_map<std::string, std::string>& flags,
                                 const std::vector<std::string>& keys)
{
    uint64_t h = 14695981039346656037ULL;
    for (const auto& k : keys)
    {
        hash_combine(h, std::hash<std::string>{}(k));
        auto it = flags.find(k);
        if (it != flags.end())
            hash_combine(h, std::hash<std::string>{}(it->second));
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

static std::unordered_map<std::string, std::string> rebuild_flags(
    const FomodInstaller& installer,
    const std::vector<std::vector<std::vector<bool>>>& selections,
    const InferenceOverrides* overrides)
{
    std::unordered_map<std::string, std::string> flags;

    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        bool visible = true;
        if (step.visible)
        {
            auto mode = ExternalConditionOverride::Unknown;
            if (overrides && si < overrides->step_visible.size())
                mode = overrides->step_visible[si];
            visible = evaluate_condition_inferred(*step.visible, flags, mode, nullptr);
        }
        if (!visible)
            continue;

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            const auto& group = step.groups[gi];
            for (size_t pi = 0; pi < group.plugins.size(); ++pi)
            {
                bool selected = false;
                if (si < selections.size() && gi < selections[si].size() &&
                    pi < selections[si][gi].size())
                {
                    selected = selections[si][gi][pi];
                }

                auto eff = evaluate_plugin_type(group.plugins[pi], flags, nullptr);
                if (eff == PluginType::Required)
                    selected = true;

                if (selected)
                {
                    for (const auto& [fn, fv] : group.plugins[pi].condition_flags)
                        flags[fn] = fv;
                }
            }
        }
    }

    return flags;
}

static std::unordered_map<std::string, std::string> rebuild_flags_before_group(
    const FomodInstaller& installer,
    const std::vector<std::vector<std::vector<bool>>>& selections,
    const InferenceOverrides* overrides,
    int stop_step_idx,
    int stop_group_idx)
{
    std::unordered_map<std::string, std::string> flags;

    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        const auto& step = installer.steps[si];
        bool visible = true;
        if (step.visible)
        {
            auto mode = ExternalConditionOverride::Unknown;
            if (overrides && si < overrides->step_visible.size())
                mode = overrides->step_visible[si];
            visible = evaluate_condition_inferred(*step.visible, flags, mode, nullptr);
        }
        if (!visible)
            continue;

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            if (static_cast<int>(si) == stop_step_idx && static_cast<int>(gi) >= stop_group_idx)
                return flags;

            const auto& group = step.groups[gi];
            for (size_t pi = 0; pi < group.plugins.size(); ++pi)
            {
                bool selected = false;
                if (si < selections.size() && gi < selections[si].size() &&
                    pi < selections[si][gi].size())
                {
                    selected = selections[si][gi][pi];
                }

                auto eff = evaluate_plugin_type(group.plugins[pi], flags, nullptr);
                if (eff == PluginType::Required)
                    selected = true;

                if (selected)
                {
                    for (const auto& [fn, fv] : group.plugins[pi].condition_flags)
                        flags[fn] = fv;
                }
            }
        }
    }

    return flags;
}

static bool step_visible_with_flags(const FomodStep& step,
                                    size_t step_idx,
                                    const std::unordered_map<std::string, std::string>& flags,
                                    const InferenceOverrides* overrides)
{
    if (!step.visible)
        return true;
    auto mode = ExternalConditionOverride::Unknown;
    if (overrides && step_idx < overrides->step_visible.size())
        mode = overrides->step_visible[step_idx];
    return evaluate_condition_inferred(*step.visible, flags, mode, nullptr);
}

using FlagSetterMap = std::unordered_map<std::string, std::vector<std::pair<int, std::string>>>;

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

static std::vector<int> compute_evidence(const FomodInstaller& installer,
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
            evidence[matches[0]] += 3;
        }
        else
        {
            for (int idx : matches)
            {
                bool hash_matched = false;
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

static ReproMetrics compare_trees(const SimulatedTree& sim,
                                  const TargetTree& target,
                                  const std::unordered_set<std::string>& excluded)
{
    ReproMetrics m;
    for (const auto& [dest, tf] : target)
    {
        if (excluded.count(dest))
            continue;

        auto it = sim.files.find(dest);
        if (it == sim.files.end())
        {
            m.missing++;
        }
        else
        {
            if (tf.size != 0 && it->second.file_size != 0 && tf.size != it->second.file_size)
                m.size_mismatch++;
            else if (tf.hash != 0 && it->second.content_hash != 0 &&
                     tf.hash != it->second.content_hash)
                m.hash_mismatch++;
            m.reproduced++;
        }
    }

    for (const auto& [dest, atom] : sim.files)
    {
        if (excluded.count(dest))
            continue;
        if (!target.count(dest))
            m.extra++;
    }
    return m;
}

static std::string format_count(int64_t n);
static std::string format_duration(int64_t seconds);
static std::string build_tqdm_bar(int64_t current,
                                  int64_t total,
                                  int64_t elapsed_s,
                                  int width = 20);

static ReproMetrics evaluate_candidate(SolverState& state,
                                       const FomodInstaller& installer,
                                       const ExpandedAtoms& atoms,
                                       const TargetTree& target,
                                       const std::unordered_set<std::string>& excluded,
                                       const InferenceOverrides* overrides)
{
    auto sim = simulate(installer, atoms, state.selections, nullptr, overrides);
    auto metrics = compare_trees(sim, target, excluded);
    state.nodes_explored++;

    // Periodic progress logging (fires for all phases: greedy, local search, backtrack)
    if (state.estimated_total > 1)
    {
        auto now = std::chrono::steady_clock::now();
        auto elapsed_ms =
            std::chrono::duration_cast<std::chrono::milliseconds>(now - state.last_progress_time)
                .count();
        if ((state.nodes_explored - state.last_progress_nodes >=
             SolverState::PROGRESS_NODE_INTERVAL) ||
            (elapsed_ms >= SolverState::PROGRESS_TIME_INTERVAL_MS &&
             state.nodes_explored > state.last_progress_nodes))
        {
            state.last_progress_nodes = state.nodes_explored;
            state.last_progress_time = now;
            int64_t pass_nodes = state.nodes_explored - state.pass_start_nodes;
            auto pass_elapsed_s =
                std::chrono::duration_cast<std::chrono::seconds>(now - state.pass_start_time)
                    .count();
            auto bar = build_tqdm_bar(pass_nodes, state.estimated_total, pass_elapsed_s);
            if (state.has_best)
            {
                Logger::instance().log(std::format("[solver] {} | best: m={} e={}",
                                                   bar,
                                                   state.best_metrics.missing,
                                                   state.best_metrics.extra));
            }
            else
            {
                Logger::instance().log(std::format("[solver] {} | no solution yet", bar));
            }
        }
    }

    if (!state.has_best || metrics.better_than(state.best_metrics))
    {
        state.best_metrics = metrics;
        state.best.selections = state.selections;
        state.best.inferred_flags = rebuild_flags(installer, state.selections, overrides);
        state.best.exact_match = metrics.exact();
        state.best.nodes_explored = state.nodes_explored;
        state.best.missing = metrics.missing;
        state.best.extra = metrics.extra;
        state.best.size_mismatch = metrics.size_mismatch;
        state.best.hash_mismatch = metrics.hash_mismatch;
        state.has_best = true;
        if (metrics.exact())
            state.found_exact = true;
    }

    return metrics;
}

static std::vector<GroupOption> generate_raw_options(
    const FomodGroup& group,
    const std::vector<int>& evidence,
    int flat_start,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap)
{
    int n = static_cast<int>(group.plugins.size());
    if (n == 0)
        return {GroupOption{}};

    std::vector<bool> required(static_cast<size_t>(n), false);
    std::vector<bool> usable(static_cast<size_t>(n), false);
    std::vector<bool> dynamic(static_cast<size_t>(n), false);
    std::vector<bool> dynamic_external(static_cast<size_t>(n), false);
    for (int i = 0; i < n; ++i)
    {
        const auto& plugin = group.plugins[static_cast<size_t>(i)];
        dynamic[static_cast<size_t>(i)] = !plugin.type_patterns.empty();
        if (dynamic[static_cast<size_t>(i)])
        {
            for (const auto& pattern : plugin.type_patterns)
            {
                if (condition_depends_on_external_state(pattern.condition))
                {
                    dynamic_external[static_cast<size_t>(i)] = true;
                    break;
                }
            }
        }

        auto eff = evaluate_plugin_type(plugin, flags, nullptr);
        if (eff == PluginType::Required)
            required[static_cast<size_t>(i)] = true;
        if (eff != PluginType::NotUsable)
            usable[static_cast<size_t>(i)] = true;
    }
    bool has_required = std::any_of(required.begin(), required.end(), [](bool b) { return b; });
    bool has_dynamic_external =
        std::any_of(dynamic_external.begin(), dynamic_external.end(), [](bool b) { return b; });

    // External file/plugin/game checks are unknown during standalone inference.
    // Keep externally-dynamic plugins selectable and avoid forcing them as
    // Required so SelectExactlyOne groups don't collapse to arbitrary options.
    if (has_dynamic_external)
    {
        for (int i = 0; i < n; ++i)
        {
            if (dynamic_external[static_cast<size_t>(i)])
                usable[static_cast<size_t>(i)] = true;

            // Preserve Required for static/flag-only semantics.
            if (required[static_cast<size_t>(i)] && dynamic_external[static_cast<size_t>(i)])
            {
                required[static_cast<size_t>(i)] = false;
            }
        }
        has_required = std::any_of(required.begin(), required.end(), [](bool b) { return b; });
    }

    std::vector<int> order(n);
    std::iota(order.begin(), order.end(), 0);
    std::sort(order.begin(),
              order.end(),
              [&](int a, int b) { return evidence[flat_start + a] > evidence[flat_start + b]; });

    auto empty_option = [&]() { return GroupOption(static_cast<size_t>(n), false); };
    auto any_selected = [](const GroupOption& opt)
    { return std::any_of(opt.begin(), opt.end(), [](bool b) { return b; }); };

    std::vector<GroupOption> options;

    switch (group.type)
    {
        case FomodGroupType::SelectAll:
        {
            auto opt = empty_option();
            for (int i = 0; i < n; ++i)
                opt[static_cast<size_t>(i)] = usable[static_cast<size_t>(i)];
            options.push_back(std::move(opt));
            break;
        }
        case FomodGroupType::SelectExactlyOne:
        {
            if (has_required)
            {
                for (int i : order)
                {
                    if (!required[static_cast<size_t>(i)])
                        continue;
                    auto opt = empty_option();
                    opt[static_cast<size_t>(i)] = true;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                for (int i : order)
                {
                    if (!usable[static_cast<size_t>(i)])
                        continue;
                    auto opt = empty_option();
                    opt[static_cast<size_t>(i)] = true;
                    options.push_back(std::move(opt));
                }
            }
            break;
        }
        case FomodGroupType::SelectAtMostOne:
        {
            if (has_required)
            {
                for (int i : order)
                {
                    if (!required[static_cast<size_t>(i)])
                        continue;
                    auto opt = empty_option();
                    opt[static_cast<size_t>(i)] = true;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                for (int i : order)
                {
                    if (!usable[static_cast<size_t>(i)])
                        continue;
                    auto opt = empty_option();
                    opt[static_cast<size_t>(i)] = true;
                    options.push_back(std::move(opt));
                }
                options.push_back(empty_option());
            }
            break;
        }
        case FomodGroupType::SelectAtLeastOne:
        case FomodGroupType::SelectAny:
        {
            bool at_least_one = (group.type == FomodGroupType::SelectAtLeastOne);
            int positive_evidence = 0;
            for (int i = 0; i < n; ++i)
            {
                if (usable[static_cast<size_t>(i)] && evidence[flat_start + i] > 0)
                    positive_evidence++;
            }

            // When SelectAny has no evidence signal, exhaustive powersets on medium-sized
            // groups blow up search space without improving reconstruction quality.
            bool force_heuristic_select_any =
                (select_any_cap > 0 && group.type == FomodGroupType::SelectAny && !has_required &&
                 positive_evidence == 0 && n >= 8);

            if (n <= 14 && !force_heuristic_select_any)
            {
                uint64_t limit = 1ULL << n;
                std::vector<std::pair<int, uint64_t>> scored;
                for (uint64_t mask = 0; mask < limit; ++mask)
                {
                    if (at_least_one && mask == 0)
                        continue;
                    bool valid = true;
                    for (int i = 0; i < n; ++i)
                    {
                        bool selected = (mask & (1ULL << i)) != 0;
                        if (required[static_cast<size_t>(i)] && !selected)
                        {
                            valid = false;
                            break;
                        }
                        if (selected && !usable[static_cast<size_t>(i)])
                        {
                            valid = false;
                            break;
                        }
                    }
                    if (!valid)
                        continue;

                    int score = 0;
                    for (int i = 0; i < n; ++i)
                        if ((mask & (1ULL << i)) != 0)
                            score += evidence[flat_start + i];
                    scored.emplace_back(score, mask);
                }

                std::sort(scored.begin(),
                          scored.end(),
                          [](const auto& a, const auto& b) { return a.first > b.first; });

                for (const auto& [_, mask] : scored)
                {
                    auto opt = empty_option();
                    for (int i = 0; i < n; ++i)
                        opt[static_cast<size_t>(i)] = (mask & (1ULL << i)) != 0;
                    options.push_back(std::move(opt));
                }
            }
            else
            {
                auto greedy = empty_option();
                for (int i = 0; i < n; ++i)
                {
                    if (required[static_cast<size_t>(i)])
                        greedy[static_cast<size_t>(i)] = true;
                    if (usable[static_cast<size_t>(i)] && evidence[flat_start + i] > 0)
                        greedy[static_cast<size_t>(i)] = true;
                }
                if (!at_least_one || any_selected(greedy))
                    options.push_back(greedy);

                for (int i : order)
                {
                    if (!greedy[static_cast<size_t>(i)] || required[static_cast<size_t>(i)])
                        continue;
                    auto var = greedy;
                    var[static_cast<size_t>(i)] = false;
                    if (at_least_one && !any_selected(var))
                        continue;
                    options.push_back(std::move(var));
                }

                for (int i : order)
                {
                    if (!usable[static_cast<size_t>(i)])
                        continue;
                    auto opt = empty_option();
                    for (int j = 0; j < n; ++j)
                        opt[static_cast<size_t>(j)] = required[static_cast<size_t>(j)];
                    opt[static_cast<size_t>(i)] = true;
                    options.push_back(std::move(opt));
                }

                if (!at_least_one)
                {
                    auto opt = empty_option();
                    for (int i = 0; i < n; ++i)
                        opt[static_cast<size_t>(i)] = required[static_cast<size_t>(i)];
                    options.push_back(std::move(opt));
                }

                // Large SelectAny groups often act as "filter" flag selectors.
                // Singletons can miss valid installs that require choosing
                // multiple filters, so keep a bounded set of pair candidates.
                bool allow_pair_candidates =
                    (group.type == FomodGroupType::SelectAny && positive_evidence > 0);
                if (allow_pair_candidates)
                {
                    std::vector<int> pair_candidates;
                    for (int i : order)
                    {
                        if (!usable[static_cast<size_t>(i)] || required[static_cast<size_t>(i)])
                            continue;
                        pair_candidates.push_back(i);
                    }

                    constexpr int kPairLimit = 8;
                    if (pair_candidates.size() > static_cast<size_t>(kPairLimit))
                        pair_candidates.resize(kPairLimit);

                    for (size_t a = 0; a < pair_candidates.size(); ++a)
                    {
                        for (size_t b = a + 1; b < pair_candidates.size(); ++b)
                        {
                            auto opt = empty_option();
                            for (int j = 0; j < n; ++j)
                                opt[static_cast<size_t>(j)] = required[static_cast<size_t>(j)];
                            opt[static_cast<size_t>(pair_candidates[a])] = true;
                            opt[static_cast<size_t>(pair_candidates[b])] = true;
                            if (at_least_one && !any_selected(opt))
                                continue;
                            options.push_back(std::move(opt));
                        }
                    }
                }
            }
            break;
        }
    }

    // Enforce intra-group dependencyType consistency using flags set by the option itself.
    // This prevents invalid combinations such as selecting a "Merged" plugin together with
    // plugins that become NotUsable when that merged flag is present.
    std::erase_if(
        options,
        [&](const GroupOption& opt)
        {
            auto local_flags = flags;
            for (int i = 0; i < n; ++i)
            {
                if (!(i < static_cast<int>(opt.size()) && opt[static_cast<size_t>(i)]))
                    continue;
                for (const auto& [fn, fv] : group.plugins[static_cast<size_t>(i)].condition_flags)
                    local_flags[fn] = fv;
            }

            for (int i = 0; i < n; ++i)
            {
                if (!(i < static_cast<int>(opt.size()) && opt[static_cast<size_t>(i)]))
                    continue;
                auto eff = evaluate_plugin_type(
                    group.plugins[static_cast<size_t>(i)], local_flags, nullptr);
                if (eff == PluginType::NotUsable && !dynamic_external[static_cast<size_t>(i)])
                    return true;
            }
            return false;
        });

    if (options.empty())
    {
        auto opt = empty_option();
        for (int i = 0; i < n; ++i)
            opt[static_cast<size_t>(i)] = required[static_cast<size_t>(i)];
        options.push_back(std::move(opt));
    }

    return options;
}

static OptionProfile build_option_profile(const GroupRef& gref,
                                          const GroupOption& option,
                                          const Precompute& pre)
{
    OptionProfile p;
    p.option = option;

    std::unordered_set<std::string> useful;
    std::unordered_set<std::string> extra;

    for (int pi = 0; pi < gref.plugin_count; ++pi)
    {
        if (!((pi < static_cast<int>(option.size())) && option[static_cast<size_t>(pi)]))
            continue;

        int flat_plugin = gref.flat_start + pi;
        p.evidence_score += pre.evidence[flat_plugin];
        p.unique_support += pre.plugin_unique_support[flat_plugin];

        const auto& step = pre.installer->steps[gref.step_idx];
        const auto& group = step.groups[gref.group_idx];
        const auto& plugin = group.plugins[static_cast<size_t>(pi)];

        for (const auto& [fn, fv] : plugin.condition_flags)
        {
            p.flags_written[fn] = fv;
            if (pre.needed_flags.count(fn))
                p.sets_needed_flag = true;
        }

        if (flat_plugin < static_cast<int>(pre.atoms->per_plugin.size()))
        {
            for (const auto& atom : pre.atoms->per_plugin[flat_plugin])
            {
                if (pre.excluded->count(atom.dest_path))
                    continue;
                p.produced.insert(atom.dest_path);
                p.produced_atoms.insert(atom.dest_path + "|" + atom.source_path);
                if (pre.target->count(atom.dest_path))
                    useful.insert(atom.dest_path);
                else
                    extra.insert(atom.dest_path);
            }
        }
    }

    p.useful_dests = static_cast<int>(useful.size());
    p.extra_dests = static_cast<int>(extra.size());
    return p;
}

static uint64_t option_signature(const OptionProfile& p)
{
    uint64_t h = 14695981039346656037ULL;

    std::vector<std::string> produced_atoms(p.produced_atoms.begin(), p.produced_atoms.end());
    std::sort(produced_atoms.begin(), produced_atoms.end());
    for (const auto& atom_sig : produced_atoms)
        hash_combine(h, std::hash<std::string>{}(atom_sig));

    std::vector<std::pair<std::string, std::string>> flags(p.flags_written.begin(),
                                                           p.flags_written.end());
    std::sort(flags.begin(), flags.end());
    for (const auto& [fn, fv] : flags)
    {
        hash_combine(h, std::hash<std::string>{}(fn));
        hash_combine(h, std::hash<std::string>{}(fv));
    }

    return h;
}

static bool better_equivalent_option(const OptionProfile& a, const OptionProfile& b)
{
    if (a.evidence_score != b.evidence_score)
        return a.evidence_score > b.evidence_score;
    if (a.unique_support != b.unique_support)
        return a.unique_support > b.unique_support;
    if (a.useful_dests != b.useful_dests)
        return a.useful_dests > b.useful_dests;
    return a.extra_dests < b.extra_dests;
}

static std::vector<GroupOption> reduce_options(const GroupRef& gref,
                                               const std::vector<GroupOption>& raw,
                                               const Precompute& pre,
                                               SolverStats& stats,
                                               int select_any_cap,
                                               bool exact_mode)
{
    if (raw.empty())
        return {};

    std::vector<OptionProfile> prof;
    prof.reserve(raw.size());
    for (const auto& opt : raw)
        prof.push_back(build_option_profile(gref, opt, pre));

    std::vector<size_t> keep;
    for (size_t i = 0; i < prof.size(); ++i)
    {
        if (!exact_mode && prof[i].extra_dests > 0 && prof[i].useful_dests == 0 &&
            !prof[i].sets_needed_flag)
        {
            stats.dropped_extra_only_options++;
            continue;
        }
        keep.push_back(i);
    }
    if (keep.empty())
    {
        keep.resize(prof.size());
        std::iota(keep.begin(), keep.end(), 0);
    }

    std::unordered_map<uint64_t, size_t> best_by_sig;
    for (size_t i : keep)
    {
        auto sig = option_signature(prof[i]);
        auto it = best_by_sig.find(sig);
        if (it == best_by_sig.end())
            best_by_sig[sig] = i;
        else
        {
            if (better_equivalent_option(prof[i], prof[it->second]))
                it->second = i;
            stats.collapsed_equivalent_options++;
        }
    }

    std::vector<size_t> candidates;
    for (const auto& [sig, idx] : best_by_sig)
        candidates.push_back(idx);

    // Keep all reduced candidates; unique support is used as ordering signal
    // but not as a hard force to avoid collapsing valid one-of choices.

    std::sort(candidates.begin(),
              candidates.end(),
              [&](size_t a, size_t b)
              {
                  if (prof[a].evidence_score != prof[b].evidence_score)
                      return prof[a].evidence_score > prof[b].evidence_score;
                  if (prof[a].unique_support != prof[b].unique_support)
                      return prof[a].unique_support > prof[b].unique_support;
                  if (prof[a].useful_dests != prof[b].useful_dests)
                      return prof[a].useful_dests > prof[b].useful_dests;
                  return prof[a].extra_dests < prof[b].extra_dests;
              });

    const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];
    bool capped_select_any =
        (group.type == FomodGroupType::SelectAny || group.type == FomodGroupType::SelectAtLeastOne);
    if (capped_select_any && select_any_cap > 0 &&
        candidates.size() > static_cast<size_t>(select_any_cap))
    {
        std::vector<size_t> narrowed;
        narrowed.reserve(static_cast<size_t>(select_any_cap));

        std::vector<bool> chosen_rank(candidates.size(), false);
        auto pick_rank = [&](size_t rank)
        {
            if (rank >= candidates.size() || chosen_rank[rank] ||
                narrowed.size() >= static_cast<size_t>(select_any_cap))
                return;
            chosen_rank[rank] = true;
            narrowed.push_back(candidates[rank]);
        };

        // Keep the best option, then diversify by number of selected plugins.
        pick_rank(0);
        std::unordered_set<int> seen_selected_counts;
        for (size_t rank = 0;
             rank < candidates.size() && narrowed.size() < static_cast<size_t>(select_any_cap);
             ++rank)
        {
            const auto& opt = prof[candidates[rank]].option;
            int selected = static_cast<int>(std::count(opt.begin(), opt.end(), true));
            if (seen_selected_counts.insert(selected).second)
                pick_rank(rank);
        }

        for (size_t rank = 0;
             rank < candidates.size() && narrowed.size() < static_cast<size_t>(select_any_cap);
             ++rank)
        {
            pick_rank(rank);
        }

        stats.capped_select_any_options += static_cast<int>(candidates.size() - narrowed.size());
        candidates = std::move(narrowed);
    }

    std::vector<GroupOption> out;
    out.reserve(candidates.size());
    for (size_t i : candidates)
        out.push_back(prof[i].option);

    if (out.empty())
        out.push_back(raw[0]);
    return out;
}

static std::string group_name(const Precompute& pre, const GroupRef& g)
{
    return std::format("step {} \"{}\" / group {} \"{}\"",
                       g.step_idx,
                       pre.installer->steps[g.step_idx].name,
                       g.group_idx,
                       pre.installer->steps[g.step_idx].groups[g.group_idx].name);
}

static const CachedOptions& get_options_for_group(
    int gidx,
    const Precompute& pre,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats)
{
    bool exact_mode = is_exact_group_mode(gidx, exact_groups);
    int effective_cap = effective_select_any_cap(gidx, select_any_cap, exact_groups);
    OptionCacheKey key{
        gidx, hash_flag_subset(flags, pre.group_cache_flags[gidx]), effective_cap, exact_mode};
    auto it = cache.find(key);
    if (it != cache.end())
        return it->second;

    const auto& gref = pre.groups[gidx];
    const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];

    auto raw = generate_raw_options(group, pre.evidence, gref.flat_start, flags, effective_cap);

    // If propagation narrowed the domain, filter out options that select eliminated plugins.
    if (pre.propagation)
    {
        const auto& domains = pre.propagation->narrowed_domains;
        if (gref.step_idx < static_cast<int>(domains.size()) &&
            gref.group_idx < static_cast<int>(domains[gref.step_idx].size()))
        {
            const auto& group_domain = domains[gref.step_idx][gref.group_idx];
            std::erase_if(raw,
                          [&](const GroupOption& opt)
                          {
                              for (size_t i = 0; i < opt.size() && i < group_domain.size(); ++i)
                                  if (opt[i] && !group_domain[i])
                                      return true;
                              return false;
                          });
        }
    }

    auto reduced = reduce_options(gref, raw, pre, stats, effective_cap, exact_mode);

    CachedOptions entry;
    entry.options = std::move(reduced);
    entry.profiles.reserve(entry.options.size());
    for (const auto& opt : entry.options)
        entry.profiles.push_back(build_option_profile(gref, opt, pre));

    auto [ins, _] = cache.emplace(key, std::move(entry));
    if (!stats.logged_group_options[gidx])
    {
        stats.logged_group_options[gidx] = true;
        int positive_evidence = 0;
        int usable_plugins = 0;
        for (int pi = 0; pi < gref.plugin_count; ++pi)
        {
            if (pre.evidence[gref.flat_start + pi] > 0)
                positive_evidence++;
            auto eff = evaluate_plugin_type(group.plugins[static_cast<size_t>(pi)], flags, nullptr);
            if (eff != PluginType::NotUsable)
                usable_plugins++;
        }
        Logger::instance().log(std::format("[solver] Branching {}: options raw={} reduced={}",
                                           group_name(pre, gref),
                                           raw.size(),
                                           ins->second.options.size()));
        Logger::instance().log(
            std::format("[solver] Group stats {}: plugins={}, usable={}, "
                        "positive_evidence={}",
                        group_name(pre, gref),
                        gref.plugin_count,
                        usable_plugins,
                        positive_evidence));
    }

    return ins->second;
}

static void link_groups(std::vector<std::unordered_set<int>>& graph, int a, int b)
{
    if (a == b)
        return;
    graph[a].insert(b);
    graph[b].insert(a);
}

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

static Precompute build_precompute(const FomodInstaller& installer,
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

static std::vector<std::string> collect_mismatched_dests(
    const SimulatedTree& sim,
    const TargetTree& target,
    const std::unordered_set<std::string>& excluded)
{
    std::unordered_set<std::string> out;

    for (const auto& [dest, tf] : target)
    {
        if (excluded.count(dest))
            continue;

        auto it = sim.files.find(dest);
        if (it == sim.files.end())
        {
            out.insert(dest);
            continue;
        }

        if (tf.size != 0 && it->second.file_size != 0 && tf.size != it->second.file_size)
            out.insert(dest);
        else if (tf.hash != 0 && it->second.content_hash != 0 && tf.hash != it->second.content_hash)
            out.insert(dest);
    }

    for (const auto& [dest, atom] : sim.files)
    {
        if (excluded.count(dest))
            continue;
        if (!target.count(dest))
            out.insert(dest);
    }

    std::vector<std::string> mismatched(out.begin(), out.end());
    std::sort(mismatched.begin(), mismatched.end());
    return mismatched;
}

static std::vector<int> groups_for_mismatches(const Precompute& pre,
                                              const std::vector<std::string>& mismatched)
{
    std::unordered_set<int> groups;
    std::vector<int> queue;

    for (const auto& dest : mismatched)
    {
        auto it = pre.dest_to_groups.find(dest);
        if (it != pre.dest_to_groups.end())
            for (int gidx : it->second)
                if (groups.insert(gidx).second)
                    queue.push_back(gidx);

        if (pre.conditional_dests.count(dest))
        {
            for (const auto& fn : pre.needed_flags)
            {
                auto sit = pre.flag_to_setter_groups.find(fn);
                if (sit == pre.flag_to_setter_groups.end())
                    continue;
                for (int gidx : sit->second)
                {
                    if (groups.insert(gidx).second)
                        queue.push_back(gidx);
                }
            }
        }
    }

    // Include prerequisite groups that set flags read by already affected groups.
    for (size_t q = 0; q < queue.size(); ++q)
    {
        int gidx = queue[q];
        if (gidx < 0 || gidx >= static_cast<int>(pre.group_reads_flags.size()))
            continue;

        for (const auto& fn : pre.group_reads_flags[gidx])
        {
            auto sit = pre.flag_to_setter_groups.find(fn);
            if (sit == pre.flag_to_setter_groups.end())
                continue;

            for (int setter_gidx : sit->second)
            {
                if (groups.insert(setter_gidx).second)
                    queue.push_back(setter_gidx);
            }
        }
    }

    std::vector<int> out(groups.begin(), groups.end());
    std::sort(out.begin(), out.end());
    return out;
}

static int selected_count(const GroupOption& option)
{
    return static_cast<int>(std::count(option.begin(), option.end(), true));
}

static std::unordered_map<int, std::vector<int>> build_repair_plugin_map(
    const SolverState& state,
    const Precompute& pre,
    const std::vector<int>& repair_groups,
    const std::vector<std::string>& mismatched)
{
    std::unordered_set<int> repair_group_set(repair_groups.begin(), repair_groups.end());
    std::unordered_map<int, std::unordered_set<int>> per_group;

    for (const auto& dest : mismatched)
    {
        auto it = pre.dest_to_plugins.find(dest);
        if (it == pre.dest_to_plugins.end())
            continue;

        for (int flat_plugin : it->second)
        {
            if (flat_plugin < 0 || flat_plugin >= static_cast<int>(pre.plugin_to_group.size()))
                continue;
            int gidx = pre.plugin_to_group[flat_plugin];
            if (repair_group_set.count(gidx) == 0)
                continue;

            const auto& gref = pre.groups[gidx];
            int local_pi = flat_plugin - gref.flat_start;
            if (local_pi < 0 || local_pi >= gref.plugin_count)
                continue;
            per_group[gidx].insert(local_pi);
        }
    }

    // Also include currently selected plugins so repair can deselect them.
    for (int gidx : repair_groups)
    {
        const auto& gref = pre.groups[gidx];
        if (gref.step_idx >= static_cast<int>(state.best.selections.size()) ||
            gref.group_idx >= static_cast<int>(state.best.selections[gref.step_idx].size()))
        {
            continue;
        }

        const auto& selected = state.best.selections[gref.step_idx][gref.group_idx];
        for (int pi = 0; pi < static_cast<int>(selected.size()); ++pi)
            if (selected[static_cast<size_t>(pi)])
                per_group[gidx].insert(pi);
    }

    std::unordered_map<int, std::vector<int>> out;
    constexpr int kMaxRepairBits = 11;  // 2048 combinations per group
    for (int gidx : repair_groups)
    {
        auto it = per_group.find(gidx);
        if (it == per_group.end())
            continue;

        const auto& gref = pre.groups[gidx];
        const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];
        if (group.type != FomodGroupType::SelectAny &&
            group.type != FomodGroupType::SelectAtLeastOne)
            continue;
        if (gref.plugin_count <= 1)
            continue;

        std::vector<int> locals(it->second.begin(), it->second.end());
        std::sort(
            locals.begin(),
            locals.end(),
            [&](int a, int b)
            { return pre.evidence[gref.flat_start + a] < pre.evidence[gref.flat_start + b]; });
        if (locals.size() > kMaxRepairBits)
            locals.resize(kMaxRepairBits);

        if (locals.size() >= 2)
            out[gidx] = std::move(locals);
    }

    return out;
}

static void targeted_repair_search(SolverState& state,
                                   const Precompute& pre,
                                   const std::vector<int>& repair_groups,
                                   const std::vector<std::string>& mismatched)
{
    if (repair_groups.empty() || mismatched.empty() || !state.has_best || state.found_exact)
        return;

    auto plugin_map = build_repair_plugin_map(state, pre, repair_groups, mismatched);
    if (plugin_map.empty())
        return;

    auto& logger = Logger::instance();
    int planned_groups = static_cast<int>(plugin_map.size());
    logger.log(std::format("[solver] Targeted repair neighborhood: {} groups", planned_groups));

    state.selections = state.best.selections;

    bool improved = true;
    int pass = 0;
    constexpr int kMaxPasses = 2;

    while (improved && !state.found_exact && pass < kMaxPasses)
    {
        improved = false;
        pass++;

        for (int gidx : repair_groups)
        {
            if (state.found_exact)
                return;

            auto mit = plugin_map.find(gidx);
            if (mit == plugin_map.end())
                continue;

            const auto& bits = mit->second;
            if (bits.size() < 2 || bits.size() > 20)
                continue;

            const auto& gref = pre.groups[gidx];
            const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];
            auto baseline = state.selections[gref.step_idx][gref.group_idx];
            auto best_group = baseline;
            auto prev_best = state.best_metrics;

            uint64_t variants = (1ULL << bits.size());
            for (uint64_t mask = 0; mask < variants; ++mask)
            {
                if (state.found_exact)
                    break;

                auto var = baseline;
                for (size_t bi = 0; bi < bits.size(); ++bi)
                {
                    int local_pi = bits[bi];
                    if (local_pi < 0 || local_pi >= static_cast<int>(var.size()))
                        continue;
                    var[static_cast<size_t>(local_pi)] = ((mask & (1ULL << bi)) != 0);
                }

                if (group.type == FomodGroupType::SelectAtLeastOne && selected_count(var) == 0)
                    continue;

                state.selections[gref.step_idx][gref.group_idx] = var;
                evaluate_candidate(
                    state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);

                if (state.best_metrics.better_than(prev_best))
                {
                    improved = true;
                    prev_best = state.best_metrics;
                    best_group = std::move(var);
                    if (state.found_exact)
                        return;
                }
            }

            state.selections[gref.step_idx][gref.group_idx] = best_group;
        }
    }

    if (state.has_best)
        state.selections = state.best.selections;
}

static bool has_remaining_group(const std::unordered_map<std::string, std::vector<int>>& map,
                                const std::string& dest,
                                const std::vector<int>& order_pos,
                                int next_idx)
{
    auto it = map.find(dest);
    if (it == map.end())
        return false;

    for (int g : it->second)
    {
        if (g >= 0 && g < static_cast<int>(order_pos.size()) && order_pos[g] >= next_idx)
            return true;
    }
    return false;
}

static ReproMetrics lower_bound(const SolverState& state,
                                const Precompute& pre,
                                const SearchPlan& plan,
                                int next_idx)
{
    ReproMetrics lb;
    auto sim = simulate(*pre.installer, *pre.atoms, state.selections, nullptr, pre.overrides);

    for (const auto& [dest, atom] : sim.files)
    {
        if (pre.excluded->count(dest))
            continue;
        if (!pre.target->count(dest))
            lb.extra++;
    }

    for (const auto& [dest, tf] : *pre.target)
    {
        if (pre.excluded->count(dest))
            continue;

        auto it = sim.files.find(dest);
        if (it == sim.files.end())
        {
            if (!has_remaining_group(pre.dest_to_groups, dest, plan.order_pos, next_idx))
                lb.missing++;
            continue;
        }

        if (tf.size != 0 && it->second.file_size != 0 && tf.size != it->second.file_size)
        {
            if (!has_remaining_group(pre.dest_to_size_match_groups, dest, plan.order_pos, next_idx))
                lb.size_mismatch++;
        }
        else if (tf.hash != 0 && it->second.content_hash != 0 && tf.hash != it->second.content_hash)
        {
            if (!has_remaining_group(
                    pre.dest_to_hash_capable_groups, dest, plan.order_pos, next_idx))
                lb.hash_mismatch++;
        }
    }

    return lb;
}

static bool cannot_beat(const ReproMetrics& lb, const ReproMetrics& best)
{
    if (lb.missing > best.missing)
        return true;
    if (lb.missing == best.missing && lb.extra > best.extra)
        return true;
    if (lb.missing == best.missing && lb.extra == best.extra &&
        lb.size_mismatch > best.size_mismatch)
        return true;
    if (lb.missing == best.missing && lb.extra == best.extra &&
        lb.size_mismatch == best.size_mismatch && lb.hash_mismatch > best.hash_mismatch)
        return true;
    return false;
}

static uint64_t contested_signature(const SolverState& state,
                                    const Precompute& pre,
                                    const SearchPlan& plan,
                                    int next_idx)
{
    uint64_t sig = 14695981039346656037ULL;

    for (int flat_plugin : pre.contested_plugins)
    {
        if (flat_plugin < 0 || flat_plugin >= static_cast<int>(pre.plugin_to_group.size()))
            continue;

        int gidx = pre.plugin_to_group[flat_plugin];
        if (gidx < 0 || gidx >= static_cast<int>(plan.order_pos.size()))
            continue;

        int pos = plan.order_pos[gidx];
        if (pos < 0 || pos >= next_idx)
            continue;

        const auto& gref = pre.groups[gidx];
        int local_pi = flat_plugin - gref.flat_start;
        if (local_pi < 0 || local_pi >= gref.plugin_count)
            continue;

        if (state.selections[gref.step_idx][gref.group_idx][local_pi])
            hash_combine(sig, static_cast<uint64_t>(flat_plugin + 1));
    }

    return sig;
}

static void apply_option(std::vector<std::vector<std::vector<bool>>>& selections,
                         const GroupRef& gref,
                         const GroupOption& option)
{
    for (int pi = 0; pi < gref.plugin_count; ++pi)
        selections[gref.step_idx][gref.group_idx][pi] =
            (pi < static_cast<int>(option.size())) && option[static_cast<size_t>(pi)];
}

static std::string format_count(int64_t n)
{
    if (n >= 1'000'000'000)
        return std::format("{:.1f}G", static_cast<double>(n) / 1e9);
    if (n >= 1'000'000)
        return std::format("{:.1f}M", static_cast<double>(n) / 1e6);
    if (n >= 1'000)
        return std::format("{:.0f}k", static_cast<double>(n) / 1e3);
    return std::to_string(n);
}

static std::string format_option_cap(int select_any_cap)
{
    if (select_any_cap <= 0)
        return "full";
    return std::to_string(select_any_cap);
}

static std::string format_duration(int64_t s)
{
    if (s < 60)
        return std::format("00:{:02}", s);
    if (s < 3600)
        return std::format("{:02}:{:02}", s / 60, s % 60);
    return std::format("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60);
}

static std::string build_tqdm_bar(int64_t current, int64_t total, int64_t elapsed_s, int width)
{
    if (total <= 0)
        total = 1;
    double ratio = std::min(1.0, static_cast<double>(current) / static_cast<double>(total));
    int filled = static_cast<int>(ratio * width);
    int pct = static_cast<int>(ratio * 100.0);

    std::string bar(width, '.');
    for (int i = 0; i < filled && i < width; ++i)
        bar[i] = '=';
    if (filled < width)
        bar[filled] = '>';

    std::string timing;
    if (elapsed_s > 0 && current > 0)
    {
        double rate = static_cast<double>(current) / static_cast<double>(elapsed_s);
        int64_t remaining_s =
            (ratio < 1.0) ? static_cast<int64_t>(static_cast<double>(total - current) / rate) : 0;
        timing = std::format(" [{}<{}, {}/s]",
                             format_duration(elapsed_s),
                             format_duration(remaining_s),
                             format_count(static_cast<int64_t>(rate)));
    }
    else
    {
        timing = std::format(" [{}<?, ?/s]", format_duration(elapsed_s));
    }

    return std::format(
        "{:>3}%|{}| {}/{} nodes{}", pct, bar, format_count(current), format_count(total), timing);
}

static void greedy_solve(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats)
{
    for (int gidx = 0; gidx < static_cast<int>(pre.groups.size()); ++gidx)
    {
        const auto& gref = pre.groups[gidx];
        const auto& step = pre.installer->steps[gref.step_idx];

        state.flags = rebuild_flags_before_group(
            *pre.installer, state.selections, pre.overrides, gref.step_idx, gref.group_idx);
        if (!step_visible_with_flags(
                step, static_cast<size_t>(gref.step_idx), state.flags, pre.overrides))
            continue;

        const auto& cached = get_options_for_group(
            gidx, pre, state.flags, select_any_cap, exact_groups, cache, stats);
        const auto& opts = cached.options;
        if (opts.empty())
            continue;

        apply_option(state.selections, gref, opts[0]);
    }

    evaluate_candidate(
        state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);
}

static void local_search(
    SolverState& state,
    const Precompute& pre,
    const std::vector<int>& order,
    int max_passes,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats)
{
    bool improved = true;
    int pass = 0;

    while (improved && !state.found_exact && pass < max_passes)
    {
        improved = false;
        pass++;

        for (int gidx : order)
        {
            if (state.found_exact)
                return;

            const auto& gref = pre.groups[gidx];
            const auto& step = pre.installer->steps[gref.step_idx];

            state.flags = rebuild_flags_before_group(
                *pre.installer, state.selections, pre.overrides, gref.step_idx, gref.group_idx);
            if (!step_visible_with_flags(
                    step, static_cast<size_t>(gref.step_idx), state.flags, pre.overrides))
                continue;

            const auto& cached = get_options_for_group(
                gidx, pre, state.flags, select_any_cap, exact_groups, cache, stats);
            const auto& opts = cached.options;
            if (opts.empty())
                continue;

            auto saved_group = state.selections[gref.step_idx][gref.group_idx];
            auto best_group = saved_group;
            auto prev_best = state.best_metrics;

            for (const auto& opt : opts)
            {
                apply_option(state.selections, gref, opt);
                evaluate_candidate(
                    state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);

                if (state.best_metrics.better_than(prev_best))
                {
                    improved = true;
                    prev_best = state.best_metrics;
                    best_group = state.selections[gref.step_idx][gref.group_idx];
                    if (state.found_exact)
                        return;
                }
            }

            state.selections[gref.step_idx][gref.group_idx] = best_group;
        }
    }
}

static void advance_flags_past_group(std::unordered_map<std::string, std::string>& flags,
                                     const FomodInstaller& installer,
                                     const std::vector<std::vector<std::vector<bool>>>& selections,
                                     const GroupRef& gref,
                                     std::vector<FlagDelta>& undo)
{
    const auto& group = installer.steps[gref.step_idx].groups[gref.group_idx];
    for (size_t pi = 0; pi < group.plugins.size(); ++pi)
    {
        bool selected = (gref.step_idx < static_cast<int>(selections.size()) &&
                         gref.group_idx < static_cast<int>(selections[gref.step_idx].size()) &&
                         pi < selections[gref.step_idx][gref.group_idx].size() &&
                         selections[gref.step_idx][gref.group_idx][pi]);
        auto eff = evaluate_plugin_type(group.plugins[pi], flags, nullptr);
        if (eff == PluginType::Required)
            selected = true;
        if (selected)
        {
            for (const auto& [fn, fv] : group.plugins[pi].condition_flags)
            {
                auto it = flags.find(fn);
                undo.push_back(
                    {fn, it != flags.end(), it != flags.end() ? it->second : std::string{}});
                flags[fn] = fv;
            }
        }
    }
}

static void undo_flags_to(std::unordered_map<std::string, std::string>& flags,
                          std::vector<FlagDelta>& undo,
                          size_t target_size)
{
    while (undo.size() > target_size)
    {
        auto& d = undo.back();
        if (d.had_value)
            flags[d.name] = std::move(d.old_value);
        else
            flags.erase(d.name);
        undo.pop_back();
    }
}

static void backtrack(SolverState& state,
                      const Precompute& pre,
                      SearchPlan& plan,
                      int next_idx,
                      int select_any_cap,
                      const std::unordered_set<int>* exact_groups,
                      std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                      SolverStats& stats)
{
    if (state.found_exact)
        return;
    if (plan.node_limit > 0 && state.nodes_explored >= plan.node_limit)
    {
        stats.pruned_node_limit++;
        return;
    }
    if (state.deadline.time_since_epoch().count() != 0 && (state.nodes_explored & 63) == 0 &&
        std::chrono::steady_clock::now() >= state.deadline)
    {
        state.deadline_exceeded = true;
        return;
    }
    if (state.deadline_exceeded)
        return;

    struct Checkpoint
    {
        int step_idx = 0;
        int group_idx = 0;
        GroupOption saved;
    };
    std::vector<Checkpoint> checkpoints;
    auto save_group = [&](const GroupRef& gref)
    {
        checkpoints.push_back(Checkpoint{
            gref.step_idx, gref.group_idx, state.selections[gref.step_idx][gref.group_idx]});
    };
    auto restore_groups = [&]()
    {
        for (auto it = checkpoints.rbegin(); it != checkpoints.rend(); ++it)
            state.selections[it->step_idx][it->group_idx] = std::move(it->saved);
    };

    // Incremental flag tracking: maintain state.flags via undo records instead of
    // rebuilding from scratch at every level.  When plan.incremental_flags is true,
    // state.flags is expected to be correct for plan.order[next_idx] on entry, and
    // is restored to that value before returning.
    std::vector<FlagDelta> flag_undo;

    int current_idx = next_idx;
    while (current_idx < static_cast<int>(plan.order.size()))
    {
        int gidx = plan.order[current_idx];
        const auto& gref = pre.groups[gidx];
        const auto& step = pre.installer->steps[gref.step_idx];

        if (!plan.incremental_flags)
            state.flags = rebuild_flags_before_group(
                *pre.installer, state.selections, pre.overrides, gref.step_idx, gref.group_idx);

        if (!step_visible_with_flags(
                step, static_cast<size_t>(gref.step_idx), state.flags, pre.overrides))
        {
            auto& cur = state.selections[gref.step_idx][gref.group_idx];
            bool needs_clear = std::any_of(cur.begin(), cur.end(), [](bool b) { return b; });
            if (needs_clear)
            {
                save_group(gref);
                std::fill(cur.begin(), cur.end(), false);
            }
            stats.skipped_invisible++;
            current_idx++;
            continue;
        }

        const auto& cached = get_options_for_group(
            gidx, pre, state.flags, select_any_cap, exact_groups, cache, stats);
        const auto& opts = cached.options;
        if (opts.size() == 1)
        {
            auto& cur = state.selections[gref.step_idx][gref.group_idx];
            if (cur != opts[0])
            {
                save_group(gref);
                cur = opts[0];
            }
            if (plan.incremental_flags)
                advance_flags_past_group(
                    state.flags, *pre.installer, state.selections, gref, flag_undo);
            current_idx++;
            continue;
        }
        break;
    }

    if (current_idx >= static_cast<int>(plan.order.size()))
    {
        evaluate_candidate(
            state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);
        if (plan.incremental_flags)
            undo_flags_to(state.flags, flag_undo, 0);
        restore_groups();
        return;
    }

    int gidx = plan.order[current_idx];
    const auto& gref = pre.groups[gidx];
    if (!plan.incremental_flags)
        state.flags = rebuild_flags_before_group(
            *pre.installer, state.selections, pre.overrides, gref.step_idx, gref.group_idx);

    int mismatch_pressure = state.best_metrics.missing + state.best_metrics.extra +
                            state.best_metrics.size_mismatch + state.best_metrics.hash_mismatch;
    bool enable_bounds = state.has_best && mismatch_pressure > 0;
    int bound_stride = 3;
    if (mismatch_pressure > 1500)
        bound_stride = 64;
    else if (mismatch_pressure > 800)
        bound_stride = 48;
    else if (mismatch_pressure > 400)
        bound_stride = 24;
    else if (mismatch_pressure > 150)
        bound_stride = 12;
    else if (mismatch_pressure > 60)
        bound_stride = 6;
    else if (mismatch_pressure > 20)
        bound_stride = 3;
    else if (mismatch_pressure > 5)
        bound_stride = 2;
    else
        bound_stride = 4;

    bool run_bounds_here = enable_bounds && current_idx >= 4 && (current_idx % bound_stride == 0);
    if (run_bounds_here)
    {
        auto lb = lower_bound(state, pre, plan, current_idx);
        if (cannot_beat(lb, state.best_metrics))
        {
            stats.pruned_lower_bound++;
            restore_groups();
            return;
        }

        bool enable_memo =
            state.best_metrics.missing <= 24 && state.best_metrics.extra <= 24 &&
            (state.best_metrics.size_mismatch + state.best_metrics.hash_mismatch) <= 24;
        if (enable_memo)
        {
            MemoKey mk;
            mk.next_idx = current_idx;
            mk.flag_state_sig = hash_flag_subset(state.flags, pre.memo_flags);
            mk.contested_sig = contested_signature(state, pre, plan, current_idx);

            auto mit = plan.memo.find(mk);
            if (mit != plan.memo.end() && !lb.better_than(mit->second))
            {
                stats.pruned_memo++;
                restore_groups();
                return;
            }
            if (mit == plan.memo.end() || lb.better_than(mit->second))
                plan.memo[mk] = lb;
        }
    }

    const auto& cached =
        get_options_for_group(gidx, pre, state.flags, select_any_cap, exact_groups, cache, stats);
    const auto& opts = cached.options;
    const auto& profiles = cached.profiles;
    auto saved_group = state.selections[gref.step_idx][gref.group_idx];
    bool exact_mode = is_exact_group_mode(gidx, exact_groups);

    for (size_t opt_idx = 0; opt_idx < opts.size(); ++opt_idx)
    {
        if (state.found_exact || state.deadline_exceeded)
            break;
        if (plan.node_limit > 0 && state.nodes_explored >= plan.node_limit)
        {
            stats.pruned_node_limit++;
            break;
        }

        const auto& profile = profiles[opt_idx];
        if (!exact_mode && profile.extra_dests > 0 && profile.useful_dests == 0 &&
            !profile.sets_needed_flag)
        {
            stats.pruned_extra_only++;
            continue;
        }

        apply_option(state.selections, gref, opts[opt_idx]);
        if (plan.incremental_flags)
        {
            size_t branch_restore = flag_undo.size();
            advance_flags_past_group(
                state.flags, *pre.installer, state.selections, gref, flag_undo);
            backtrack(
                state, pre, plan, current_idx + 1, select_any_cap, exact_groups, cache, stats);
            undo_flags_to(state.flags, flag_undo, branch_restore);
        }
        else
        {
            backtrack(
                state, pre, plan, current_idx + 1, select_any_cap, exact_groups, cache, stats);
        }
    }

    state.selections[gref.step_idx][gref.group_idx] = saved_group;
    if (plan.incremental_flags)
        undo_flags_to(state.flags, flag_undo, 0);
    restore_groups();
}

static uint64_t estimate_search_space(
    const Precompute& pre,
    const std::vector<int>& order,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats,
    uint64_t limit)
{
    uint64_t total = 1;
    for (int gidx : order)
    {
        const auto& cached =
            get_options_for_group(gidx, pre, flags, select_any_cap, exact_groups, cache, stats);
        uint64_t c = std::max<uint64_t>(1ULL, static_cast<uint64_t>(cached.options.size()));
        if (total > limit / c)
            return limit + 1;
        total *= c;
    }
    return total;
}

static void run_backtrack_pass(
    SolverState& state,
    const Precompute& pre,
    const std::vector<int>& order,
    int node_limit,
    const std::string& label,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats)
{
    if (order.empty() || state.found_exact)
        return;

    SearchPlan plan;
    plan.order = order;
    plan.node_limit = node_limit;
    plan.order_pos.assign(pre.groups.size(), -1);
    for (int i = 0; i < static_cast<int>(order.size()); ++i)
        plan.order_pos[order[i]] = i;

    // Enable incremental flag tracking when the order covers all groups (no gap
    // groups to worry about).  This turns O(N^2) flag-rebuild-per-leaf into O(N).
    plan.incremental_flags = (order.size() == pre.groups.size());
    if (plan.incremental_flags && !order.empty())
    {
        const auto& first_gref = pre.groups[order[0]];
        state.flags = rebuild_flags_before_group(*pre.installer,
                                                 state.selections,
                                                 pre.overrides,
                                                 first_gref.step_idx,
                                                 first_gref.group_idx);
    }

    auto space = estimate_search_space(
        pre, order, state.flags, select_any_cap, exact_groups, cache, stats, 1'000'000'000ULL);
    state.estimated_total =
        (node_limit > 0) ? static_cast<int64_t>(node_limit) : static_cast<int64_t>(space);
    state.last_progress_nodes = state.nodes_explored;
    state.pass_start_nodes = state.nodes_explored;
    state.pass_start_time = std::chrono::steady_clock::now();
    state.last_progress_time = state.pass_start_time;

    if (node_limit == 0)
    {
        Logger::instance().log(
            std::format("[solver] Phase: backtrack {} ({} groups, space={}, select_any_cap={})",
                        label,
                        order.size(),
                        format_count(static_cast<int64_t>(space)),
                        format_option_cap(select_any_cap)));
    }
    else
    {
        Logger::instance().log(
            std::format("[solver] Phase: backtrack {} ({} groups, limit={}, select_any_cap={})",
                        label,
                        order.size(),
                        format_count(node_limit),
                        format_option_cap(select_any_cap)));
    }

    // Log initial 0% bar so even fast solves have a visible start
    if (state.estimated_total > 1)
    {
        auto bar = build_tqdm_bar(0, state.estimated_total, 0);
        Logger::instance().log(std::format("[solver] {} | searching...", bar));
    }

    backtrack(state, pre, plan, 0, select_any_cap, exact_groups, cache, stats);

    // Log a final 100% bar
    int64_t pass_nodes = state.nodes_explored - state.pass_start_nodes;
    if (pass_nodes > 0 && state.estimated_total > 1 &&
        state.nodes_explored > state.last_progress_nodes)
    {
        auto now = std::chrono::steady_clock::now();
        auto pass_elapsed_s =
            std::chrono::duration_cast<std::chrono::seconds>(now - state.pass_start_time).count();
        // Show actual nodes explored as 100% (denominator = actual, not estimate)
        auto bar = build_tqdm_bar(pass_nodes, pass_nodes, pass_elapsed_s);
        if (state.has_best)
        {
            Logger::instance().log(std::format("[solver] {} | best: m={} e={} (done)",
                                               bar,
                                               state.best_metrics.missing,
                                               state.best_metrics.extra));
        }
        else
        {
            Logger::instance().log(std::format("[solver] {} | no solution (done)", bar));
        }
    }

    // Reset estimated_total so stale data doesn't leak to the next phase
    state.estimated_total = 0;
}

SolverResult solve_fomod_csp(const FomodInstaller& installer,
                             const ExpandedAtoms& atoms,
                             const AtomIndex& atom_index,
                             const TargetTree& target,
                             const std::unordered_set<std::string>& excluded_dests,
                             const InferenceOverrides* overrides,
                             const PropagationResult* propagation)
{
    auto& logger = Logger::instance();

    std::vector<GroupRef> groups;
    int flat_plugins = 0;
    for (int si = 0; si < static_cast<int>(installer.steps.size()); ++si)
    {
        for (int gi = 0; gi < static_cast<int>(installer.steps[si].groups.size()); ++gi)
        {
            int pc = static_cast<int>(installer.steps[si].groups[gi].plugins.size());
            groups.push_back({si, gi, flat_plugins, pc});
            flat_plugins += pc;
        }
    }

    auto group_priority = [](FomodGroupType t) -> int
    {
        switch (t)
        {
            case FomodGroupType::SelectAll:
                return 4;
            case FomodGroupType::SelectExactlyOne:
                return 3;
            case FomodGroupType::SelectAtMostOne:
                return 2;
            case FomodGroupType::SelectAtLeastOne:
                return 1;
            case FomodGroupType::SelectAny:
                return 0;
        }
        return 0;
    };

    for (int si = 0; si < static_cast<int>(installer.steps.size()); ++si)
    {
        int begin = -1;
        int end = -1;
        for (int i = 0; i < static_cast<int>(groups.size()); ++i)
        {
            if (groups[i].step_idx == si)
            {
                if (begin < 0)
                    begin = i;
                end = i + 1;
            }
        }

        if (begin < 0)
            continue;

        std::sort(groups.begin() + begin,
                  groups.begin() + end,
                  [&](const GroupRef& a, const GroupRef& b)
                  {
                      const auto& ga = installer.steps[a.step_idx].groups[a.group_idx];
                      const auto& gb = installer.steps[b.step_idx].groups[b.group_idx];
                      int pa = group_priority(ga.type);
                      int pb = group_priority(gb.type);
                      if (pa != pb)
                          return pa > pb;
                      return a.plugin_count < b.plugin_count;
                  });
    }

    auto evidence = compute_evidence(installer, atoms, atom_index, target, excluded_dests);
    auto pre = build_precompute(installer,
                                atoms,
                                atom_index,
                                target,
                                excluded_dests,
                                overrides,
                                propagation,
                                groups,
                                evidence);

    SolverState state;
    state.selections.resize(installer.steps.size());
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        state.selections[si].resize(installer.steps[si].groups.size());
        for (size_t gi = 0; gi < installer.steps[si].groups.size(); ++gi)
            state.selections[si][gi].assign(installer.steps[si].groups[gi].plugins.size(), false);
    }

    static constexpr int kSolverTimeLimitSeconds = 600;
    state.deadline =
        std::chrono::steady_clock::now() + std::chrono::seconds(kSolverTimeLimitSeconds);

    SolverStats stats;
    stats.logged_group_options.assign(pre.groups.size(), false);
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash> options_cache;
    int select_any_cap = kSelectAnyCapNarrow;

    logger.log(std::format("[solver] Starting CSP: {} groups, {} total plugins, {} components",
                           pre.groups.size(),
                           flat_plugins,
                           pre.components.size()));

    logger.log(std::format("[solver] Phase: greedy ({} groups)", pre.groups.size()));
    greedy_solve(state, pre, select_any_cap, nullptr, options_cache, stats);
    logger.log(
        std::format("[solver] After greedy: exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
                    state.found_exact,
                    state.best.missing,
                    state.best.extra,
                    state.best.size_mismatch,
                    state.best.hash_mismatch));

    if (!state.found_exact)
    {
        std::vector<int> all_groups(pre.groups.size());
        std::iota(all_groups.begin(), all_groups.end(), 0);
        logger.log(std::format("[solver] Phase: local search ({} groups)", all_groups.size()));
        local_search(state, pre, all_groups, 5, select_any_cap, nullptr, options_cache, stats);

        logger.log(std::format(
            "[solver] After local search: exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
            state.found_exact,
            state.best.missing,
            state.best.extra,
            state.best.size_mismatch,
            state.best.hash_mismatch));

        if (!state.found_exact && state.has_best)
        {
            auto sim_best = simulate(installer, atoms, state.best.selections, nullptr, overrides);
            auto mismatched = collect_mismatched_dests(sim_best, target, excluded_dests);
            auto affected = groups_for_mismatches(pre, mismatched);
            std::string affected_groups;
            for (size_t i = 0; i < affected.size(); ++i)
            {
                if (i > 0)
                    affected_groups += "; ";
                affected_groups += group_name(pre, pre.groups[affected[i]]);
            }
            logger.log(std::format("[solver] Remaining mismatches: {} dests, {} affected groups",
                                   mismatched.size(),
                                   affected.size()));
            if (!affected_groups.empty())
                logger.log(std::format("[solver] Mismatch-affecting groups: {}", affected_groups));

            if (!state.found_exact)
            {
                targeted_repair_search(state, pre, affected, mismatched);
                logger.log(
                    std::format("[solver] After targeted repair: exact={}, missing={}, "
                                "extra={}, size_mm={}, hash_mm={}",
                                state.found_exact,
                                state.best.missing,
                                state.best.extra,
                                state.best.size_mismatch,
                                state.best.hash_mismatch));
            }
        }
    }

    if (!state.found_exact && !state.deadline_exceeded && pre.components.size() > 1)
    {
        logger.log(
            std::format("[solver] Component decomposition: {} components", pre.components.size()));

        int comp_idx = 0;
        for (const auto& comp : pre.components)
        {
            if (state.found_exact || state.deadline_exceeded)
                break;
            if (comp.empty())
            {
                comp_idx++;
                continue;
            }

            comp_idx++;
            logger.log(std::format("[solver] Phase: backtrack component {}/{} ({} groups)",
                                   comp_idx,
                                   pre.components.size(),
                                   comp.size()));

            if (state.has_best)
                state.selections = state.best.selections;
            state.flags = rebuild_flags(installer, state.selections, overrides);

            local_search(state, pre, comp, 2, select_any_cap, nullptr, options_cache, stats);
            if (state.found_exact)
                break;

            auto space = estimate_search_space(pre,
                                               comp,
                                               state.flags,
                                               select_any_cap,
                                               nullptr,
                                               options_cache,
                                               stats,
                                               10'000'000ULL);
            int limit = (space <= 2'000'000ULL) ? 0 : 2'000'000;
            run_backtrack_pass(state,
                               pre,
                               comp,
                               limit,
                               "component",
                               select_any_cap,
                               nullptr,
                               options_cache,
                               stats);
        }

        logger.log(
            std::format("[solver] After component solve: exact={}, missing={}, extra={}, "
                        "size_mm={}, hash_mm={}",
                        state.found_exact,
                        state.best.missing,
                        state.best.extra,
                        state.best.size_mismatch,
                        state.best.hash_mismatch));
    }

    if (!state.found_exact && !state.deadline_exceeded && state.has_best)
    {
        bool near_perfect = (state.best.missing == 0 && state.best.extra == 0 &&
                             state.best.size_mismatch <= 1 && state.best.hash_mismatch <= 2);
        if (near_perfect)
        {
            auto sim_best = simulate(installer, atoms, state.best.selections, nullptr, overrides);
            auto mismatched = collect_mismatched_dests(sim_best, target, excluded_dests);
            auto repair_groups = groups_for_mismatches(pre, mismatched);

            if (!repair_groups.empty() && repair_groups.size() < pre.groups.size())
            {
                logger.log(std::format(
                    "[solver] Residual repair mode: {} mismatched dests, {} affected groups",
                    mismatched.size(),
                    repair_groups.size()));
                std::string group_list;
                for (size_t i = 0; i < repair_groups.size(); ++i)
                {
                    if (i > 0)
                        group_list += "; ";
                    group_list += group_name(pre, pre.groups[repair_groups[i]]);
                }
                logger.log(
                    std::format("[solver] Residual mismatch-affecting groups: {}", group_list));

                state.selections = state.best.selections;
                state.flags = rebuild_flags(installer, state.selections, overrides);
                local_search(
                    state, pre, repair_groups, 3, select_any_cap, nullptr, options_cache, stats);
                run_backtrack_pass(state,
                                   pre,
                                   repair_groups,
                                   3'000'000,
                                   "residual",
                                   select_any_cap,
                                   nullptr,
                                   options_cache,
                                   stats);
            }
        }
    }

    if (!state.found_exact && !state.deadline_exceeded && state.has_best)
    {
        auto sim_best = simulate(installer, atoms, state.best.selections, nullptr, overrides);
        auto mismatched = collect_mismatched_dests(sim_best, target, excluded_dests);
        auto focus_groups = groups_for_mismatches(pre, mismatched);

        if (!focus_groups.empty() && focus_groups.size() < pre.groups.size())
        {
            logger.log(std::format("[solver] Focused search: {} mismatched dests, {} groups",
                                   mismatched.size(),
                                   focus_groups.size()));
            state.selections = state.best.selections;
            state.flags = rebuild_flags(installer, state.selections, overrides);
            local_search(
                state, pre, focus_groups, 2, select_any_cap, nullptr, options_cache, stats);

            if (!state.found_exact)
            {
                auto space = estimate_search_space(pre,
                                                   focus_groups,
                                                   state.flags,
                                                   select_any_cap,
                                                   nullptr,
                                                   options_cache,
                                                   stats,
                                                   10'000'000ULL);
                int limit = (space <= 6'000'000ULL) ? 0 : 6'000'000;
                run_backtrack_pass(state,
                                   pre,
                                   focus_groups,
                                   limit,
                                   "focused",
                                   select_any_cap,
                                   nullptr,
                                   options_cache,
                                   stats);
            }

            if (!state.found_exact)
            {
                std::unordered_set<int> exact_focus_groups(focus_groups.begin(),
                                                           focus_groups.end());
                logger.log(
                    std::format("[solver] Focused exact fallback: {} groups", focus_groups.size()));

                state.selections = state.best.selections;
                state.flags = rebuild_flags(installer, state.selections, overrides);
                auto exact_space = estimate_search_space(pre,
                                                         focus_groups,
                                                         state.flags,
                                                         select_any_cap,
                                                         &exact_focus_groups,
                                                         options_cache,
                                                         stats,
                                                         12'000'000ULL);
                int exact_limit = (exact_space <= 8'000'000ULL) ? 0 : 8'000'000;
                run_backtrack_pass(state,
                                   pre,
                                   focus_groups,
                                   exact_limit,
                                   "focused-exact",
                                   select_any_cap,
                                   &exact_focus_groups,
                                   options_cache,
                                   stats);
            }

            logger.log(std::format(
                "[solver] After focused search: exact={}, missing={}, extra={}, size_mm={}, "
                "hash_mm={}",
                state.found_exact,
                state.best.missing,
                state.best.extra,
                state.best.size_mismatch,
                state.best.hash_mismatch));
        }
    }

    if (!state.found_exact && !state.deadline_exceeded)
    {
        if (state.has_best)
            state.selections = state.best.selections;
        state.flags = rebuild_flags(installer, state.selections, overrides);

        std::vector<int> base_order;
        for (const auto& comp : pre.components)
            for (int gidx : comp)
                base_order.push_back(gidx);
        if (base_order.empty())
        {
            base_order.resize(pre.groups.size());
            std::iota(base_order.begin(), base_order.end(), 0);
        }
        std::sort(base_order.begin(), base_order.end());

        // Keep the global pass in canonical order. Reordering individual groups can
        // evaluate flag-gated steps before prerequisite groups and permanently skip them.
        const std::vector<int>& global_order = base_order;

        struct GlobalPassOutcome
        {
            bool hit_limit = false;
            int capped_options = 0;
        };

        auto run_global_pass = [&](int cap,
                                   const std::string& label,
                                   const std::unordered_set<int>* exact_groups =
                                       nullptr) -> GlobalPassOutcome
        {
            if (state.found_exact || state.deadline_exceeded)
                return {};

            if (state.has_best)
                state.selections = state.best.selections;
            state.flags = rebuild_flags(installer, state.selections, overrides);

            options_cache.clear();
            auto search_space = estimate_search_space(pre,
                                                      global_order,
                                                      state.flags,
                                                      cap,
                                                      exact_groups,
                                                      options_cache,
                                                      stats,
                                                      10'000'000ULL);
            int node_limit = (search_space <= 10'000'000ULL) ? 0 : 10'000'000;
            if (cap == kSelectAnyCapFull)
            {
                int full_limit = 2'000'000;
                if (state.has_best && (state.best.missing > 0 || state.best.extra > 0))
                    full_limit = 6'000'000;
                if (node_limit == 0)
                    node_limit = full_limit;
                else
                    node_limit = std::min(node_limit, full_limit);
            }

            int capped_before = stats.capped_select_any_options;
            int pruned_before = stats.pruned_node_limit;
            run_backtrack_pass(state,
                               pre,
                               global_order,
                               node_limit,
                               label,
                               cap,
                               exact_groups,
                               options_cache,
                               stats);
            return {
                stats.pruned_node_limit > pruned_before,
                stats.capped_select_any_options - capped_before,
            };
        };

        auto narrow = run_global_pass(kSelectAnyCapNarrow, "global");
        auto medium = GlobalPassOutcome{};
        if (!state.found_exact)
        {
            logger.log(std::format("[solver] Option widening: SelectAny cap {} -> {}",
                                   format_option_cap(kSelectAnyCapNarrow),
                                   format_option_cap(kSelectAnyCapMedium)));
            medium = run_global_pass(kSelectAnyCapMedium, "global-widened");
        }

        bool capped_any = (narrow.capped_options > 0 || medium.capped_options > 0);
        bool unresolved_after_medium =
            !state.found_exact && state.has_best &&
            (state.best.missing > 0 || state.best.extra > 0 || state.best.size_mismatch > 0 ||
             state.best.hash_mismatch > 0);
        bool need_full_fallback =
            !state.found_exact && (medium.hit_limit || capped_any || unresolved_after_medium);
        if (need_full_fallback)
        {
            if (!state.found_exact && state.has_best)
            {
                auto sim_best =
                    simulate(installer, atoms, state.best.selections, nullptr, overrides);
                auto mismatched = collect_mismatched_dests(sim_best, target, excluded_dests);
                auto affected = groups_for_mismatches(pre, mismatched);
                if (!affected.empty() && affected.size() < pre.groups.size())
                {
                    logger.log(
                        std::format("[solver] Global targeted fallback: {} mismatched dests, {} "
                                    "affected groups",
                                    mismatched.size(),
                                    affected.size()));
                    std::unordered_set<int> exact_groups(affected.begin(), affected.end());
                    run_global_pass(kSelectAnyCapMedium, "global-targeted", &exact_groups);
                }
            }

            bool unresolved_after_targeted =
                !state.found_exact && state.has_best &&
                (state.best.missing > 0 || state.best.extra > 0 || state.best.size_mismatch > 0 ||
                 state.best.hash_mismatch > 0);
            if (unresolved_after_targeted)
            {
                logger.log(std::format("[solver] Option widening: SelectAny cap {} -> {}",
                                       format_option_cap(kSelectAnyCapMedium),
                                       format_option_cap(kSelectAnyCapFull)));
                run_global_pass(kSelectAnyCapFull, "global-full");
            }
        }
    }

    if (state.deadline_exceeded)
        logger.log(std::format("[solver] Wall-clock time limit ({}s) exceeded after {} nodes",
                               kSolverTimeLimitSeconds,
                               state.nodes_explored));

    if (state.has_best)
    {
        state.best.nodes_explored = state.nodes_explored;
        logger.log(std::format(
            "[solver] Done: {} nodes, exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
            state.nodes_explored,
            state.best.exact_match,
            state.best.missing,
            state.best.extra,
            state.best.size_mismatch,
            state.best.hash_mismatch));
    }
    else
    {
        logger.log(
            std::format("[solver] No solution found ({} nodes explored)", state.nodes_explored));
        state.best.nodes_explored = state.nodes_explored;
    }

    logger.log(
        std::format("[solver] Pruning summary: extra_only={}, lower_bound={}, memo={}, "
                    "invisible_skip={}, node_limit={}",
                    stats.pruned_extra_only,
                    stats.pruned_lower_bound,
                    stats.pruned_memo,
                    stats.skipped_invisible,
                    stats.pruned_node_limit));

    logger.log(
        std::format("[solver] Domain reduction summary: dropped_extra_only_options={}, "
                    "collapsed_equivalent={}, forced_unique={}, capped_select_any={}",
                    stats.dropped_extra_only_options,
                    stats.collapsed_equivalent_options,
                    stats.forced_unique_options,
                    stats.capped_select_any_options));

    return state.best;
}

}  // namespace mo2core

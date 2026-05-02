#include "FomodCSPSolver.h"
#include "FomodCSPOptions.h"
#include "FomodCSPPrecompute.h"
#include "FomodCSPSolverInternal.h"
#include "FomodCSPTypes.h"
#include "FomodDependencyEvaluator.h"
#include "FomodForwardSimulator.h"
#include "FomodPropagator.h"
#include "Logger.h"

#include <algorithm>
#include <cassert>
#include <chrono>
#include <cstdint>
#include <format>
#include <numeric>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

// ---------------------------------------------------------------------------
// Solver tuning constants (SolverConfig is defined in FomodCSPSolverInternal.h)
// ---------------------------------------------------------------------------

constexpr SolverConfig kConfig{};
static constexpr size_t kMaxMemoEntries = 100'000;

// ---------------------------------------------------------------------------

// Rebuild the flag map by replaying all selected plugins up to a given group.
// When stop_step_idx < 0, processes the entire installer (full rebuild).
// When stop_step_idx >= 0, stops before the specified (step, group) pair.
std::unordered_map<std::string, std::string> rebuild_flags(
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
            {
                mode = overrides->step_visible[si];
            }
            visible = evaluate_condition_inferred(*step.visible, flags, mode, nullptr);
        }
        if (!visible)
        {
            continue;
        }

        for (size_t gi = 0; gi < step.groups.size(); ++gi)
        {
            if (stop_step_idx >= 0 && static_cast<int>(si) == stop_step_idx &&
                static_cast<int>(gi) >= stop_group_idx)
            {
                return flags;
            }

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
                {
                    selected = true;
                }

                if (selected)
                {
                    for (const auto& [fn, fv] : group.plugins[pi].condition_flags)
                    {
                        flags[fn] = fv;
                    }
                }
            }
        }
    }

    return flags;
}

bool step_visible_with_flags(const FomodStep& step,
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

// Shared tree comparison: walks sim vs target, calling predicates to decide
// whether each mismatch should be counted. compare_trees passes always-true;
// lower_bound passes has_remaining_group checks.
template <typename MissingPred, typename SizePred, typename HashPred>
static ReproMetrics compare_trees_impl(const SimulatedTree& sim,
                                       const TargetTree& target,
                                       const std::unordered_set<std::string>& excluded,
                                       MissingPred&& on_missing,
                                       SizePred&& on_size_mismatch,
                                       HashPred&& on_hash_mismatch)
{
    ReproMetrics m;
    for (const auto& [dest, tf] : target)
    {
        if (excluded.count(dest))
        {
            continue;
        }

        auto it = sim.files.find(dest);
        if (it == sim.files.end())
        {
            if (on_missing(dest))
            {
                m.missing++;
            }
            continue;
        }

        if (tf.size != 0 && it->second.file_size != 0 && tf.size != it->second.file_size)
        {
            if (on_size_mismatch(dest))
            {
                m.size_mismatch++;
            }
        }
        else if (tf.hash != 0 && it->second.content_hash != 0 && tf.hash != it->second.content_hash)
        {
            if (on_hash_mismatch(dest))
            {
                m.hash_mismatch++;
            }
        }
        else
        {
            m.reproduced++;
        }
    }

    for (const auto& [dest, atom] : sim.files)
    {
        if (excluded.count(dest))
        {
            continue;
        }
        if (!target.count(dest))
        {
            m.extra++;
        }
    }
    return m;
}

static ReproMetrics compare_trees(const SimulatedTree& sim,
                                  const TargetTree& target,
                                  const std::unordered_set<std::string>& excluded)
{
    auto always_count = [](const std::string&) { return true; };
    return compare_trees_impl(sim, target, excluded, always_count, always_count, always_count);
}

std::string format_count(int64_t n);
static std::string format_duration(int64_t seconds);
static std::string build_tqdm_bar(int64_t current,
                                  int64_t total,
                                  int64_t elapsed_s,
                                  int width = 20);

// Forward declarations for incremental flag tracking (defined after local_search).
static void advance_flags_past_group(std::unordered_map<std::string, std::string>& flags,
                                     const FomodInstaller& installer,
                                     const std::vector<std::vector<std::vector<bool>>>& selections,
                                     const GroupRef& gref,
                                     std::vector<FlagDelta>& undo);

ReproMetrics evaluate_candidate(SolverState& state,
                                const FomodInstaller& installer,
                                const ExpandedAtoms& atoms,
                                const TargetTree& target,
                                const std::unordered_set<std::string>& excluded,
                                const InferenceOverrides* overrides)
{
    // Reuse a thread-local scratch buffer to avoid repeated unordered_map
    // allocation on this hot path (called millions of times per solve).
    thread_local SimulatedTree scratch_sim;
    simulate_into(scratch_sim, installer, atoms, state.search.selections, nullptr, overrides);
    auto metrics = compare_trees(scratch_sim, target, excluded);

    // Periodic memory release for long-running solves on thread-pool servers.
    static constexpr size_t kScratchShrinkThreshold = 50'000;
    if (scratch_sim.files.size() > kScratchShrinkThreshold)
    {
        scratch_sim.files.clear();
        scratch_sim.files.rehash(0);
    }

    state.search.nodes_explored++;

    // Periodic progress logging (fires for all phases: greedy, local search, backtrack)
    if (state.progress.estimated_total > 1)
    {
        auto now = std::chrono::steady_clock::now();
        auto elapsed_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                              now - state.progress.last_progress_time)
                              .count();
        if ((state.search.nodes_explored - state.progress.last_progress_nodes >=
             SolverProgress::PROGRESS_NODE_INTERVAL) ||
            (elapsed_ms >= SolverProgress::PROGRESS_TIME_INTERVAL_MS &&
             state.search.nodes_explored > state.progress.last_progress_nodes))
        {
            state.progress.last_progress_nodes = state.search.nodes_explored;
            state.progress.last_progress_time = now;
            int64_t pass_nodes = state.search.nodes_explored - state.progress.pass_start_nodes;
            auto pass_elapsed_s = std::chrono::duration_cast<std::chrono::seconds>(
                                      now - state.progress.pass_start_time)
                                      .count();
            auto bar = build_tqdm_bar(pass_nodes, state.progress.estimated_total, pass_elapsed_s);
            if (state.best.has_best)
            {
                Logger::instance().log(std::format("[solver] {} | best: m={} e={}",
                                                   bar,
                                                   state.best.best_metrics.missing,
                                                   state.best.best_metrics.extra));
            }
            else
            {
                Logger::instance().log(std::format("[solver] {} | no solution yet", bar));
            }
        }
    }

    if (!state.best.has_best || metrics.better_than(state.best.best_metrics))
    {
        state.best.best_metrics = metrics;
        state.best.best.selections = state.search.selections;
        state.best.best.inferred_flags =
            rebuild_flags(installer, state.search.selections, overrides);
        state.best.best.exact_match = metrics.exact();
        state.best.best.nodes_explored = state.search.nodes_explored;
        state.best.best.missing = metrics.missing;
        state.best.best.extra = metrics.extra;
        state.best.best.size_mismatch = metrics.size_mismatch;
        state.best.best.hash_mismatch = metrics.hash_mismatch;
        state.best.has_best = true;
        if (metrics.exact())
            state.search.found_exact = true;
    }

    return metrics;
}

// Collect all destination paths where the simulation diverges from the target.
// Three mismatch categories:
//   - Missing: dest is in target but not produced by the simulation.
//   - Size/hash mismatch: dest exists in both but the file content differs
//     (wrong source variant selected).
//   - Extra: dest is produced by the simulation but absent from the target
//     (an unwanted plugin was selected or a conditional pattern fired wrongly).
std::vector<std::string> collect_mismatched_dests(const SimulatedTree& sim,
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
            out.insert(dest);  // missing
            continue;
        }

        if (tf.size != 0 && it->second.file_size != 0 && tf.size != it->second.file_size)
            out.insert(dest);  // size mismatch
        else if (tf.hash != 0 && it->second.content_hash != 0 && tf.hash != it->second.content_hash)
            out.insert(dest);  // hash mismatch
    }

    for (const auto& [dest, atom] : sim.files)
    {
        if (excluded.count(dest))
            continue;
        if (!target.count(dest))
            out.insert(dest);  // extra
    }

    std::vector<std::string> mismatched(out.begin(), out.end());
    std::sort(mismatched.begin(), mismatched.end());
    return mismatched;
}

// Find all groups whose selection could influence the given mismatched dests.
//
// Uses a BFS-style expansion through the flag dependency chain:
//   1. Seed: groups that directly produce any mismatched dest path.
//   2. Seed: for conditional dests (produced by flag-gated patterns), include
//      all groups that set flags needed by those conditional patterns.
//   3. Expand: for every affected group, find the flags it reads, then include
//      the groups that set those flags. Repeat until no new groups are added.
//      The queue doubles as the visited-set worklist, so expansion terminates
//      when all transitive flag dependencies have been traced.
//
// This produces the minimal "repair neighborhood" -- only groups that could
// change the mismatched output need to be re-searched.
std::vector<int> groups_for_mismatches(const Precompute& pre,
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

    // BFS expansion: include prerequisite groups that set flags read by
    // already-affected groups. The queue grows during iteration.
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
        if (gref.step_idx >= static_cast<int>(state.best.best.selections.size()) ||
            gref.group_idx >= static_cast<int>(state.best.best.selections[gref.step_idx].size()))
        {
            continue;
        }

        const auto& selected = state.best.best.selections[gref.step_idx][gref.group_idx];
        for (int pi = 0; pi < static_cast<int>(selected.size()); ++pi)
            if (selected[static_cast<size_t>(pi)])
                per_group[gidx].insert(pi);
    }

    std::unordered_map<int, std::vector<int>> out;
    // Cap at 11 toggle bits per group: 2^11 = 2048 combos is the empirical
    // sweet spot -- enough to cover typical multi-select groups without making
    // the repair pass itself combinatorially expensive.
    constexpr int kMaxRepairBits = 11;
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

// Targeted repair: exhaustive bit-flip search over a small plugin neighborhood.
//
// Strategy: for each repair group (SelectAny/AtLeastOne only), identify the
// plugins that produce mismatched dests plus those currently selected. Enumerate
// all 2^k on/off combinations of those plugins (capped at k=11 => 2048 combos
// per group). Run up to 2 passes: the second pass picks up improvements enabled
// by cross-group flag changes from pass 1. This is much cheaper than full
// backtracking but often resolves remaining mismatches after greedy/local search.
void targeted_repair_search(SolverState& state,
                            const Precompute& pre,
                            const std::vector<int>& repair_groups,
                            const std::vector<std::string>& mismatched)
{
    if (repair_groups.empty() || mismatched.empty() || !state.best.has_best ||
        state.search.found_exact)
        return;

    auto plugin_map = build_repair_plugin_map(state, pre, repair_groups, mismatched);
    if (plugin_map.empty())
        return;

    auto& logger = Logger::instance();
    int planned_groups = static_cast<int>(plugin_map.size());
    logger.log(std::format("[solver] Targeted repair neighborhood: {} groups", planned_groups));

    state.search.selections = state.best.best.selections;

    bool improved = true;
    int pass = 0;
    constexpr int kMaxPasses = 2;

    while (improved && !state.search.found_exact && pass < kMaxPasses)
    {
        improved = false;
        pass++;

        for (int gidx : repair_groups)
        {
            if (state.search.found_exact)
                return;

            auto mit = plugin_map.find(gidx);
            if (mit == plugin_map.end())
                continue;

            const auto& bits = mit->second;
            if (bits.size() < 2 || bits.size() > 20)
                continue;

            const auto& gref = pre.groups[gidx];
            const auto& group = pre.installer->steps[gref.step_idx].groups[gref.group_idx];
            auto baseline = state.search.selections[gref.step_idx][gref.group_idx];
            auto best_group = baseline;
            auto prev_best = state.best.best_metrics;

            uint64_t variants = (1ULL << bits.size());
            for (uint64_t mask = 0; mask < variants; ++mask)
            {
                if (state.search.found_exact)
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

                state.search.selections[gref.step_idx][gref.group_idx] = var;
                evaluate_candidate(
                    state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);

                if (state.best.best_metrics.better_than(prev_best))
                {
                    improved = true;
                    prev_best = state.best.best_metrics;
                    best_group = std::move(var);
                    if (state.search.found_exact)
                        return;
                }
            }

            state.search.selections[gref.step_idx][gref.group_idx] = best_group;
        }
    }

    if (state.best.has_best)
        state.search.selections = state.best.best.selections;
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
    thread_local SimulatedTree scratch_lb;
    simulate_into(
        scratch_lb, *pre.installer, *pre.atoms, state.search.selections, nullptr, pre.overrides);

    auto can_fix_missing = [&](const std::string& dest)
    { return !has_remaining_group(pre.dest_to_groups, dest, plan.order_pos, next_idx); };
    auto can_fix_size = [&](const std::string& dest)
    { return !has_remaining_group(pre.dest_to_size_match_groups, dest, plan.order_pos, next_idx); };
    auto can_fix_hash = [&](const std::string& dest)
    {
        return !has_remaining_group(
            pre.dest_to_hash_capable_groups, dest, plan.order_pos, next_idx);
    };

    return compare_trees_impl(
        scratch_lb, *pre.target, *pre.excluded, can_fix_missing, can_fix_size, can_fix_hash);
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

        if (gref.step_idx >= static_cast<int>(state.search.selections.size()) ||
            gref.group_idx >= static_cast<int>(state.search.selections[gref.step_idx].size()) ||
            local_pi >=
                static_cast<int>(state.search.selections[gref.step_idx][gref.group_idx].size()))
            continue;

        if (state.search.selections[gref.step_idx][gref.group_idx][local_pi])
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

std::string format_count(int64_t n)
{
    if (n >= 1'000'000'000)
        return std::format("{:.1f}G", static_cast<double>(n) / 1e9);
    if (n >= 1'000'000)
        return std::format("{:.1f}M", static_cast<double>(n) / 1e6);
    if (n >= 1'000)
        return std::format("{:.0f}k", static_cast<double>(n) / 1e3);
    return std::to_string(n);
}

std::string format_option_cap(int select_any_cap)
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

void greedy_solve(SolverState& state,
                  const Precompute& pre,
                  int select_any_cap,
                  const std::unordered_set<int>* exact_groups,
                  std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                  SolverStats& stats)
{
    // Use incremental flag tracking instead of rebuild_flags per group.
    // Groups in pre.groups are in document order, so advancing one-by-one
    // produces the same flag state as rebuild_flags (O(N) vs O(N^2)).
    state.search.flags.clear();
    std::vector<FlagDelta> flag_undo;
    int prev_step = -1;
    bool prev_step_visible = false;

    for (int gidx = 0; gidx < static_cast<int>(pre.groups.size()); ++gidx)
    {
        const auto& gref = pre.groups[gidx];
        const auto& step = pre.installer->steps[gref.step_idx];

        // Cache step visibility per step transition (all groups in a step share visibility).
        // rebuild_flags checks visibility once per step using flags from prior steps.
        if (gref.step_idx != prev_step)
        {
            prev_step = gref.step_idx;
            prev_step_visible = step_visible_with_flags(
                step, static_cast<size_t>(gref.step_idx), state.search.flags, pre.overrides);
        }

        if (!prev_step_visible)
            continue;

        const auto& cached = get_options_for_group(
            gidx, pre, state.search.flags, select_any_cap, exact_groups, cache, stats);
        const auto& opts = cached.options;
        if (opts.empty())
        {
            // Still advance flags past this group (Required-type plugins may contribute flags)
            advance_flags_past_group(
                state.search.flags, *pre.installer, state.search.selections, gref, flag_undo);
            continue;
        }

        apply_option(state.search.selections, gref, opts[0]);
        advance_flags_past_group(
            state.search.flags, *pre.installer, state.search.selections, gref, flag_undo);
    }

    evaluate_candidate(
        state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);
}

void local_search(SolverState& state,
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

    while (improved && !state.search.found_exact && pass < max_passes)
    {
        improved = false;
        pass++;

        for (int gidx : order)
        {
            if (state.search.found_exact)
                return;

            const auto& gref = pre.groups[gidx];
            const auto& step = pre.installer->steps[gref.step_idx];

            state.search.flags = rebuild_flags(*pre.installer,
                                               state.search.selections,
                                               pre.overrides,
                                               gref.step_idx,
                                               gref.group_idx);
            if (!step_visible_with_flags(
                    step, static_cast<size_t>(gref.step_idx), state.search.flags, pre.overrides))
                continue;

            const auto& cached = get_options_for_group(
                gidx, pre, state.search.flags, select_any_cap, exact_groups, cache, stats);
            const auto& opts = cached.options;
            if (opts.empty())
                continue;

            auto saved_group = state.search.selections[gref.step_idx][gref.group_idx];
            auto best_group = saved_group;
            auto prev_best = state.best.best_metrics;

            for (const auto& opt : opts)
            {
                apply_option(state.search.selections, gref, opt);
                evaluate_candidate(
                    state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);

                if (state.best.best_metrics.better_than(prev_best))
                {
                    improved = true;
                    prev_best = state.best.best_metrics;
                    best_group = state.search.selections[gref.step_idx][gref.group_idx];
                    if (state.search.found_exact)
                        return;
                }
            }

            state.search.selections[gref.step_idx][gref.group_idx] = best_group;
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

struct CheckpointGuard
{
    struct Entry
    {
        int step_idx = 0;
        int group_idx = 0;
        GroupOption saved;
    };
    std::vector<Entry> entries;
    std::vector<std::vector<std::vector<bool>>>& selections;

    explicit CheckpointGuard(std::vector<std::vector<std::vector<bool>>>& sel)
        : selections(sel)
    {
    }
    ~CheckpointGuard() { restore(); }
    CheckpointGuard(const CheckpointGuard&) = delete;
    CheckpointGuard& operator=(const CheckpointGuard&) = delete;

    void restore()
    {
        for (auto it = entries.rbegin(); it != entries.rend(); ++it)
            selections[it->step_idx][it->group_idx] = std::move(it->saved);
        entries.clear();
    }

    bool save(const GroupRef& gref, size_t limit)
    {
        if (entries.size() >= limit)
        {
            Logger::instance().log_warning("[solver] Checkpoint limit reached, abandoning branch");
            return false;
        }
        entries.push_back(
            {gref.step_idx, gref.group_idx, selections[gref.step_idx][gref.group_idx]});
        return true;
    }
};

// Iterative backtracking search using an explicit stack to avoid deep recursion
// that could overflow the MSVC default 1 MB stack on large FOMOD installers.
// The max stack depth equals the number of multi-option groups in the search
// order, which is typically 10-50 but can reach hundreds for pathological inputs.
static constexpr size_t kMaxBacktrackDepth = 500;

static void backtrack(SolverState& state,
                      const Precompute& pre,
                      SearchPlan& plan,
                      int start_idx,
                      int select_any_cap,
                      const std::unordered_set<int>* exact_groups,
                      std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                      SolverStats& stats)
{
    // Per-level state stored on the heap instead of the call stack.
    struct Frame
    {
        int next_idx;             // initial group position for this level
        int branch_idx;           // resolved position of the multi-option group (-1 = pending)
        int gidx;                 // global group index for the branching group
        size_t opt_idx;           // next option to try (0..N)
        GroupOption saved_group;  // group state before branching
        size_t flag_mark;         // flag_undo size at frame entry
        size_t skip_flag_mark;    // flag_undo size after skip phase (before option advance)
        size_t checkpoint_mark;   // checkpoints size at frame entry
    };

    // Shared across all levels (replaces per-frame vectors).
    struct CheckpointEntry
    {
        int step_idx = 0;
        int group_idx = 0;
        GroupOption saved;
    };

    std::vector<CheckpointEntry> checkpoints;
    std::vector<FlagDelta> flag_undo;
    std::vector<Frame> stack;
    stack.reserve(64);

    auto save_checkpoint = [&](const GroupRef& gref) -> bool
    {
        if (checkpoints.size() >= kConfig.max_checkpoints)
        {
            Logger::instance().log_warning("[solver] Checkpoint limit reached, abandoning branch");
            return false;
        }
        checkpoints.push_back({gref.step_idx,
                               gref.group_idx,
                               state.search.selections[gref.step_idx][gref.group_idx]});
        return true;
    };

    auto restore_checkpoints_to = [&](size_t mark)
    {
        while (checkpoints.size() > mark)
        {
            auto& e = checkpoints.back();
            state.search.selections[e.step_idx][e.group_idx] = std::move(e.saved);
            checkpoints.pop_back();
        }
    };

    // Unwind the current frame: restore flags, checkpoints, and the branching group.
    auto unwind_frame = [&](Frame& f)
    {
        if (f.branch_idx >= 0)
        {
            const auto& gref = pre.groups[f.gidx];
            state.search.selections[gref.step_idx][gref.group_idx] = std::move(f.saved_group);
        }
        if (plan.incremental_flags)
            undo_flags_to(state.search.flags, flag_undo, f.flag_mark);
        restore_checkpoints_to(f.checkpoint_mark);
    };

    auto should_bail = [&]() -> bool
    { return state.search.found_exact || state.progress.deadline_exceeded; };

    stack.push_back(
        {start_idx, -1, 0, 0, {}, flag_undo.size(), flag_undo.size(), checkpoints.size()});

    while (!stack.empty())
    {
        if (should_bail())
        {
            while (!stack.empty())
            {
                unwind_frame(stack.back());
                stack.pop_back();
            }
            return;
        }

        auto& f = stack.back();

        // ------------------------------------------------------------------
        // Phase 1: Initialize frame -- skip single-option groups, run bounds
        // ------------------------------------------------------------------
        if (f.branch_idx < 0)
        {
            if (stack.size() > kMaxBacktrackDepth)
            {
                if (stats.max_depth_aborts == 0)
                {
                    Logger::instance().log_warning(
                        std::format("[solver] kMaxBacktrackDepth ({}) exceeded; abandoning "
                                    "branch (logged once per solve)",
                                    kMaxBacktrackDepth));
                }
                stats.max_depth_aborts++;
                unwind_frame(f);
                stack.pop_back();
                continue;
            }
            if (plan.node_limit > 0 && state.search.nodes_explored >= plan.node_limit)
            {
                stats.pruned_node_limit++;
                unwind_frame(f);
                stack.pop_back();
                continue;
            }
            if (state.progress.deadline.time_since_epoch().count() != 0 &&
                (state.search.nodes_explored & 63) == 0 &&
                std::chrono::steady_clock::now() >= state.progress.deadline)
            {
                state.progress.deadline_exceeded = true;
                unwind_frame(f);
                stack.pop_back();
                continue;
            }

            f.flag_mark = flag_undo.size();
            f.checkpoint_mark = checkpoints.size();

            // Skip through single-option and invisible groups.
            int ci = f.next_idx;
            bool bail = false;
            while (ci < static_cast<int>(plan.order.size()))
            {
                int gidx = plan.order[ci];
                const auto& gref = pre.groups[gidx];
                const auto& step = pre.installer->steps[gref.step_idx];

                if (!plan.incremental_flags)
                    state.search.flags = rebuild_flags(*pre.installer,
                                                       state.search.selections,
                                                       pre.overrides,
                                                       gref.step_idx,
                                                       gref.group_idx);

                if (!step_visible_with_flags(step,
                                             static_cast<size_t>(gref.step_idx),
                                             state.search.flags,
                                             pre.overrides))
                {
                    auto& cur = state.search.selections[gref.step_idx][gref.group_idx];
                    bool needs_clear =
                        std::any_of(cur.begin(), cur.end(), [](bool b) { return b; });
                    if (needs_clear)
                    {
                        if (!save_checkpoint(gref))
                        {
                            bail = true;
                            break;
                        }
                        if (state.progress.deadline_exceeded)
                        {
                            bail = true;
                            break;
                        }
                        std::fill(cur.begin(), cur.end(), false);
                    }
                    stats.skipped_invisible++;
                    ci++;
                    continue;
                }

                const auto& cached = get_options_for_group(
                    gidx, pre, state.search.flags, select_any_cap, exact_groups, cache, stats);
                if (cached.options.size() == 1)
                {
                    auto& cur = state.search.selections[gref.step_idx][gref.group_idx];
                    if (cur != cached.options[0])
                    {
                        if (!save_checkpoint(gref))
                        {
                            bail = true;
                            break;
                        }
                        if (state.progress.deadline_exceeded)
                        {
                            bail = true;
                            break;
                        }
                        cur = cached.options[0];
                    }
                    if (plan.incremental_flags)
                        advance_flags_past_group(state.search.flags,
                                                 *pre.installer,
                                                 state.search.selections,
                                                 gref,
                                                 flag_undo);
                    ci++;
                    continue;
                }
                break;  // multi-option group found
            }

            if (bail)
            {
                unwind_frame(f);
                stack.pop_back();
                continue;
            }

            // All groups processed -- evaluate this leaf.
            if (ci >= static_cast<int>(plan.order.size()))
            {
                evaluate_candidate(
                    state, *pre.installer, *pre.atoms, *pre.target, *pre.excluded, pre.overrides);
                unwind_frame(f);
                stack.pop_back();
                continue;
            }

            // Bounds checking and memoization for the branching group.
            int gidx = plan.order[ci];
            const auto& gref = pre.groups[gidx];
            if (!plan.incremental_flags)
                state.search.flags = rebuild_flags(*pre.installer,
                                                   state.search.selections,
                                                   pre.overrides,
                                                   gref.step_idx,
                                                   gref.group_idx);

            int mismatch_pressure =
                state.best.best_metrics.missing + state.best.best_metrics.extra +
                state.best.best_metrics.size_mismatch + state.best.best_metrics.hash_mismatch;
            bool enable_bounds = state.best.has_best && mismatch_pressure > 0;
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

            bool run_bounds_here = enable_bounds && ci >= 4 && (ci % bound_stride == 0);
            if (run_bounds_here)
            {
                auto lb = lower_bound(state, pre, plan, ci);
                if (cannot_beat(lb, state.best.best_metrics))
                {
                    stats.pruned_lower_bound++;
                    unwind_frame(f);
                    stack.pop_back();
                    continue;
                }

                bool enable_memo = state.best.best_metrics.missing <= 24 &&
                                   state.best.best_metrics.extra <= 24 &&
                                   (state.best.best_metrics.size_mismatch +
                                    state.best.best_metrics.hash_mismatch) <= 24;
                if (enable_memo)
                {
                    MemoKey mk;
                    mk.next_idx = ci;
                    mk.flag_state_sig = hash_flag_subset(state.search.flags, pre.memo_flags);
                    mk.contested_sig = contested_signature(state, pre, plan, ci);

                    auto mit = plan.memo.find(mk);
                    if (mit != plan.memo.end() && !lb.better_than(mit->second))
                    {
                        stats.pruned_memo++;
                        unwind_frame(f);
                        stack.pop_back();
                        continue;
                    }
                    if (mit == plan.memo.end() || lb.better_than(mit->second))
                    {
                        if (plan.memo.size() >= kMaxMemoEntries)
                            plan.memo.clear();
                        plan.memo[mk] = lb;
                    }
                }
            }

            // Prepare for branching.
            f.branch_idx = ci;
            f.gidx = gidx;
            f.saved_group = state.search.selections[gref.step_idx][gref.group_idx];
            f.skip_flag_mark = flag_undo.size();
            f.opt_idx = 0;
        }

        // ------------------------------------------------------------------
        // Phase 2: Try the next option for the branching group
        // ------------------------------------------------------------------
        const auto& gref = pre.groups[f.gidx];
        const auto& cached = get_options_for_group(
            f.gidx, pre, state.search.flags, select_any_cap, exact_groups, cache, stats);
        const auto& opts = cached.options;
        const auto& profiles = cached.profiles;

        // Undo the previous option's flag advance (if any) back to skip-phase state.
        if (plan.incremental_flags && f.opt_idx > 0)
            undo_flags_to(state.search.flags, flag_undo, f.skip_flag_mark);

        // Find the next viable option.
        bool exact_mode = is_exact_group_mode(f.gidx, exact_groups);
        bool found_option = false;
        while (f.opt_idx < opts.size())
        {
            if (should_bail())
                break;
            if (plan.node_limit > 0 && state.search.nodes_explored >= plan.node_limit)
            {
                stats.pruned_node_limit++;
                break;
            }

            const auto& profile = profiles[f.opt_idx];
            if (!exact_mode && profile.extra_dests > 0 && profile.useful_dests == 0 &&
                !profile.sets_needed_flag)
            {
                stats.pruned_extra_only++;
                f.opt_idx++;
                continue;
            }

            found_option = true;
            break;
        }

        if (!found_option)
        {
            // All options exhausted -- unwind and pop.
            // Ensure flags are at skip-phase state before full unwind.
            if (plan.incremental_flags)
                undo_flags_to(state.search.flags, flag_undo, f.skip_flag_mark);
            unwind_frame(f);
            stack.pop_back();
            continue;
        }

        // Apply this option and push a child frame.
        apply_option(state.search.selections, gref, opts[f.opt_idx]);
        f.opt_idx++;

        if (plan.incremental_flags)
            advance_flags_past_group(
                state.search.flags, *pre.installer, state.search.selections, gref, flag_undo);

        stack.push_back({f.branch_idx + 1,
                         -1,
                         0,
                         0,
                         {},
                         flag_undo.size(),
                         flag_undo.size(),
                         checkpoints.size()});
    }
}

uint64_t estimate_search_space(
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

void run_backtrack_pass(
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
    if (order.empty() || state.search.found_exact)
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
        state.search.flags = rebuild_flags(*pre.installer,
                                           state.search.selections,
                                           pre.overrides,
                                           first_gref.step_idx,
                                           first_gref.group_idx);
    }

    auto space = estimate_search_space(pre,
                                       order,
                                       state.search.flags,
                                       select_any_cap,
                                       exact_groups,
                                       cache,
                                       stats,
                                       kConfig.greedy_space_cap);
    state.progress.estimated_total =
        (node_limit > 0) ? static_cast<int64_t>(node_limit) : static_cast<int64_t>(space);
    state.progress.last_progress_nodes = state.search.nodes_explored;
    state.progress.pass_start_nodes = state.search.nodes_explored;
    state.progress.pass_start_time = std::chrono::steady_clock::now();
    state.progress.last_progress_time = state.progress.pass_start_time;

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
    if (state.progress.estimated_total > 1)
    {
        auto bar = build_tqdm_bar(0, state.progress.estimated_total, 0);
        Logger::instance().log(std::format("[solver] {} | searching...", bar));
    }

    backtrack(state, pre, plan, 0, select_any_cap, exact_groups, cache, stats);

    // Log a final 100% bar
    int64_t pass_nodes = state.search.nodes_explored - state.progress.pass_start_nodes;
    if (pass_nodes > 0 && state.progress.estimated_total > 1 &&
        state.search.nodes_explored > state.progress.last_progress_nodes)
    {
        auto now = std::chrono::steady_clock::now();
        auto pass_elapsed_s =
            std::chrono::duration_cast<std::chrono::seconds>(now - state.progress.pass_start_time)
                .count();
        // Show actual nodes explored as 100% (denominator = actual, not estimate)
        auto bar = build_tqdm_bar(pass_nodes, pass_nodes, pass_elapsed_s);
        if (state.best.has_best)
        {
            Logger::instance().log(std::format("[solver] {} | best: m={} e={} (done)",
                                               bar,
                                               state.best.best_metrics.missing,
                                               state.best.best_metrics.extra));
        }
        else
        {
            Logger::instance().log(std::format("[solver] {} | no solution (done)", bar));
        }
    }

    // Reset estimated_total so stale data doesn't leak to the next phase
    state.progress.estimated_total = 0;
}

// Phase functions are in FomodCSPSolverPhases.cpp; declarations in FomodCSPSolverInternal.h.

// ---------------------------------------------------------------------------

SolverResult solve_fomod_csp(const FomodInstaller& installer,
                             const ExpandedAtoms& atoms,
                             const AtomIndex& atom_index,
                             const TargetTree& target,
                             const std::unordered_set<std::string>& excluded_dests,
                             const InferenceOverrides* overrides,
                             const PropagationResult* propagation)
{
    auto& logger = Logger::instance();

    // Build group list and sort by priority within each step.
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

    // Build precomputed data structures.
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

    // Initialize solver state.
    SolverState state;
    state.search.selections.resize(installer.steps.size());
    for (size_t si = 0; si < installer.steps.size(); ++si)
    {
        state.search.selections[si].resize(installer.steps[si].groups.size());
        for (size_t gi = 0; gi < installer.steps[si].groups.size(); ++gi)
            state.search.selections[si][gi].assign(installer.steps[si].groups[gi].plugins.size(),
                                                   false);
    }

    // Verify that every precomputed GroupRef indexes validly into the
    // selections array.  The indices are guaranteed by construction (both
    // the installer and pre.groups are built from the same data), but an
    // out-of-bounds access here would be undefined behavior in hot loops
    // that intentionally skip per-access bounds checks for performance.
    for (const auto& g : pre.groups)
    {
        assert(g.step_idx >= 0);
        assert(static_cast<size_t>(g.step_idx) < state.search.selections.size());
        assert(g.group_idx >= 0);
        assert(static_cast<size_t>(g.group_idx) <
               state.search.selections[static_cast<size_t>(g.step_idx)].size());
        assert(g.plugin_count >= 0);
        assert(
            static_cast<size_t>(g.flat_start + g.plugin_count) <=
            state.search
                    .selections[static_cast<size_t>(g.step_idx)][static_cast<size_t>(g.group_idx)]
                    .size() +
                static_cast<size_t>(g.flat_start));
    }

    state.progress.deadline =
        std::chrono::steady_clock::now() + std::chrono::seconds(kConfig.time_limit_seconds);

    SolverStats stats;
    stats.logged_group_options.assign(pre.groups.size(), false);
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash> options_cache;
    int select_any_cap = kSelectAnyCapNarrow;

    logger.log(std::format("[solver] Starting CSP: {} groups, {} total plugins, {} components",
                           pre.groups.size(),
                           flat_plugins,
                           pre.components.size()));

    // Track which phases actually ran so the diagnostic builder can attribute
    // the result to the deepest phase that contributed. Phase 1 always runs.
    bool ran_phase2 = false;
    bool ran_phase3 = false;
    bool ran_phase4 = false;
    bool ran_phase5 = false;

    // Phase 1: greedy + local search + targeted repair
    run_initial_phases(state, pre, select_any_cap, options_cache, stats);

    // Phase 2: component decomposition
    if (!state.search.found_exact && !state.progress.deadline_exceeded)
    {
        ran_phase2 = true;
        run_component_decomposition(state, pre, select_any_cap, options_cache, stats);
    }

    // Phase 3: residual repair (near-perfect cleanup)
    if (!state.search.found_exact && !state.progress.deadline_exceeded)
    {
        ran_phase3 = true;
        run_residual_repair(state, pre, select_any_cap, options_cache, stats);
    }

    // Phase 4: mismatch-focused search
    if (!state.search.found_exact && !state.progress.deadline_exceeded)
    {
        ran_phase4 = true;
        run_focused_search(state, pre, select_any_cap, options_cache, stats);
    }

    // Phase 5: global fallback with widening
    if (!state.search.found_exact && !state.progress.deadline_exceeded)
    {
        ran_phase5 = true;
        run_global_fallback(state, pre, options_cache, stats);
    }

    // Final reporting.
    if (state.progress.deadline_exceeded)
        logger.log(std::format("[solver] Wall-clock time limit ({}s) exceeded after {} nodes",
                               kConfig.time_limit_seconds,
                               state.search.nodes_explored));

    if (state.best.has_best)
    {
        state.best.best.nodes_explored = state.search.nodes_explored;
        logger.log(std::format(
            "[solver] Done: {} nodes, exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
            state.search.nodes_explored,
            state.best.best.exact_match,
            state.best.best.missing,
            state.best.best.extra,
            state.best.best.size_mismatch,
            state.best.best.hash_mismatch));
    }
    else
    {
        logger.log(std::format("[solver] No solution found ({} nodes explored)",
                               state.search.nodes_explored));
        state.best.best.nodes_explored = state.search.nodes_explored;
    }

    logger.log(
        std::format("[solver] Pruning summary: extra_only={}, lower_bound={}, memo={}, "
                    "invisible_skip={}, node_limit={}, max_depth_aborts={}",
                    stats.pruned_extra_only,
                    stats.pruned_lower_bound,
                    stats.pruned_memo,
                    stats.skipped_invisible,
                    stats.pruned_node_limit,
                    stats.max_depth_aborts));

    logger.log(
        std::format("[solver] Domain reduction summary: dropped_extra_only_options={}, "
                    "collapsed_equivalent={}, forced_unique={}, capped_select_any={}",
                    stats.dropped_extra_only_options,
                    stats.collapsed_equivalent_options,
                    stats.forced_unique_options,
                    stats.capped_select_any_options));

    // Populate diagnostic fields. The CSP solver attributes the result to the
    // deepest phase that ran; per-group attribution mirrors that, except for
    // groups already resolved by propagation (those carry an empty phase).
    std::string final_phase = "csp.greedy";
    if (ran_phase5)
    {
        final_phase = "csp.fallback";
    }
    else if (ran_phase4)
    {
        final_phase = "csp.focused";
    }
    else if (ran_phase3)
    {
        final_phase = "csp.repair";
    }
    else if (ran_phase2)
    {
        final_phase = "csp.local_search";
    }
    state.best.best.phase_reached = final_phase;

    state.best.best.phase_per_group.resize(installer.steps.size());
    state.best.best.alternatives_per_group.resize(installer.steps.size());
    for (size_t s = 0; s < installer.steps.size(); ++s)
    {
        state.best.best.phase_per_group[s].resize(installer.steps[s].groups.size());
        state.best.best.alternatives_per_group[s].assign(installer.steps[s].groups.size(), 0);
        for (size_t g = 0; g < installer.steps[s].groups.size(); ++g)
        {
            bool prop_resolved = false;
            if (propagation)
            {
                for (const auto& [ps, pg] : propagation->resolved_groups)
                {
                    if (ps == static_cast<int>(s) && pg == static_cast<int>(g))
                    {
                        prop_resolved = true;
                        break;
                    }
                }
            }
            state.best.best.phase_per_group[s][g] = prop_resolved ? "" : final_phase;
        }
    }

    return state.best.best;
}

}  // namespace mo2core

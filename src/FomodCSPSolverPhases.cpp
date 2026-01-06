#include "FomodCSPOptions.h"
#include "FomodCSPSolverInternal.h"
#include "FomodForwardSimulator.h"
#include "Logger.h"

#include <algorithm>
#include <format>
#include <numeric>
#include <unordered_set>
#include <vector>

namespace mo2core
{

// ---------------------------------------------------------------------------
// Phase functions extracted from solve_fomod_csp
//
// Phase ordering rationale:
//   1. Greedy + local search + targeted repair -- cheap O(N) and O(N*k)
//      passes that solve the majority of FOMOD installers outright. Most
//      installers are linear flag chains with little ambiguity.
//   2. Component decomposition -- exploit the fact that many groups are
//      independent (no shared dests or flag links). Solving each component
//      separately turns an exponential search into a product of smaller ones.
//   3. Residual repair -- when only 1-2 mismatches remain (size/hash), a
//      narrow targeted search is cheaper than a global pass.
//   4. Focused search -- re-examine only groups that contribute to remaining
//      mismatches, with increasing exact-mode caps for thorough coverage.
//   5. Global fallback -- progressively wider backtrack passes (narrow ->
//      medium -> full SelectAny caps) over all groups, used only when earlier
//      phases leave unresolved mismatches.
//
// Each phase exits early when an exact match is found.
// ---------------------------------------------------------------------------

// Phase 1: Greedy solve, local search, and first targeted repair.
void run_initial_phases(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats)
{
    auto& logger = Logger::instance();

    logger.log(std::format("[solver] Phase: greedy ({} groups)", pre.groups.size()));
    greedy_solve(state, pre, select_any_cap, nullptr, options_cache, stats);
    logger.log(
        std::format("[solver] After greedy: exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
                    state.search.found_exact,
                    state.best.best.missing,
                    state.best.best.extra,
                    state.best.best.size_mismatch,
                    state.best.best.hash_mismatch));

    if (state.search.found_exact)
        return;

    std::vector<int> all_groups(pre.groups.size());
    std::iota(all_groups.begin(), all_groups.end(), 0);
    logger.log(std::format("[solver] Phase: local search ({} groups)", all_groups.size()));
    local_search(state, pre, all_groups, 5, select_any_cap, nullptr, options_cache, stats);

    logger.log(std::format(
        "[solver] After local search: exact={}, missing={}, extra={}, size_mm={}, hash_mm={}",
        state.search.found_exact,
        state.best.best.missing,
        state.best.best.extra,
        state.best.best.size_mismatch,
        state.best.best.hash_mismatch));

    if (state.search.found_exact || !state.best.has_best)
        return;

    auto sim_best =
        simulate(*pre.installer, *pre.atoms, state.best.best.selections, nullptr, pre.overrides);
    auto mismatched = collect_mismatched_dests(sim_best, *pre.target, *pre.excluded);
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

    if (!state.search.found_exact)
    {
        targeted_repair_search(state, pre, affected, mismatched);
        logger.log(
            std::format("[solver] After targeted repair: exact={}, missing={}, "
                        "extra={}, size_mm={}, hash_mm={}",
                        state.search.found_exact,
                        state.best.best.missing,
                        state.best.best.extra,
                        state.best.best.size_mismatch,
                        state.best.best.hash_mismatch));
    }
}

// Phase 2: Component decomposition -- per-component local search + backtrack.
void run_component_decomposition(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats)
{
    if (pre.components.size() <= 1)
        return;

    auto& logger = Logger::instance();
    logger.log(
        std::format("[solver] Component decomposition: {} components", pre.components.size()));

    int comp_idx = 0;
    for (const auto& comp : pre.components)
    {
        if (state.search.found_exact || state.progress.deadline_exceeded)
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

        if (state.best.has_best)
            state.search.selections = state.best.best.selections;
        state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);

        // 2 local search passes per component: enough to converge on small
        // subproblems without dominating solve time on the many components.
        local_search(state, pre, comp, 2, select_any_cap, nullptr, options_cache, stats);
        if (state.search.found_exact)
            break;

        auto space = estimate_search_space(pre,
                                           comp,
                                           state.search.flags,
                                           select_any_cap,
                                           nullptr,
                                           options_cache,
                                           stats,
                                           kConfig.component_space_cap);
        int limit = (space <= static_cast<uint64_t>(kConfig.component_node_limit))
                        ? 0
                        : kConfig.component_node_limit;
        run_backtrack_pass(
            state, pre, comp, limit, "component", select_any_cap, nullptr, options_cache, stats);
    }

    logger.log(
        std::format("[solver] After component solve: exact={}, missing={}, extra={}, "
                    "size_mm={}, hash_mm={}",
                    state.search.found_exact,
                    state.best.best.missing,
                    state.best.best.extra,
                    state.best.best.size_mismatch,
                    state.best.best.hash_mismatch));
}

// Phase 3: Residual repair -- near-perfect cleanup when m=0, e=0.
void run_residual_repair(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats)
{
    if (!state.best.has_best)
        return;

    // "Near perfect" thresholds: <=1 size mismatch and <=2 hash mismatches.
    // Size mismatches are rare in well-formed FOMODs (usually 0 or 1 from a
    // single wrong variant), so 1 is a tight but sufficient bound. Hash
    // mismatches are more common (mod authors repack files with trivial edits),
    // so 2 allows for typical noise without triggering expensive global search.
    bool near_perfect = (state.best.best.missing == 0 && state.best.best.extra == 0 &&
                         state.best.best.size_mismatch <= 1 && state.best.best.hash_mismatch <= 2);
    if (!near_perfect)
        return;

    auto sim_best =
        simulate(*pre.installer, *pre.atoms, state.best.best.selections, nullptr, pre.overrides);
    auto mismatched = collect_mismatched_dests(sim_best, *pre.target, *pre.excluded);
    auto repair_groups = groups_for_mismatches(pre, mismatched);

    if (repair_groups.empty() || repair_groups.size() >= pre.groups.size())
        return;

    auto& logger = Logger::instance();
    logger.log(std::format("[solver] Residual repair mode: {} mismatched dests, {} affected groups",
                           mismatched.size(),
                           repair_groups.size()));
    std::string group_list;
    for (size_t i = 0; i < repair_groups.size(); ++i)
    {
        if (i > 0)
            group_list += "; ";
        group_list += group_name(pre, pre.groups[repair_groups[i]]);
    }
    logger.log(std::format("[solver] Residual mismatch-affecting groups: {}", group_list));

    state.search.selections = state.best.best.selections;
    state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);
    // 3 passes for residual repair: one more than component-level because the
    // repair set is small and inter-group flag effects may need an extra round.
    local_search(state, pre, repair_groups, 3, select_any_cap, nullptr, options_cache, stats);
    run_backtrack_pass(state,
                       pre,
                       repair_groups,
                       kConfig.residual_node_limit,
                       "residual",
                       select_any_cap,
                       nullptr,
                       options_cache,
                       stats);
}

// Phase 4: Mismatch-focused local search + backtrack.
void run_focused_search(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats)
{
    if (!state.best.has_best)
        return;

    auto sim_best =
        simulate(*pre.installer, *pre.atoms, state.best.best.selections, nullptr, pre.overrides);
    auto mismatched = collect_mismatched_dests(sim_best, *pre.target, *pre.excluded);
    auto focus_groups = groups_for_mismatches(pre, mismatched);

    if (focus_groups.empty() || focus_groups.size() >= pre.groups.size())
        return;

    auto& logger = Logger::instance();
    logger.log(std::format("[solver] Focused search: {} mismatched dests, {} groups",
                           mismatched.size(),
                           focus_groups.size()));
    state.search.selections = state.best.best.selections;
    state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);
    local_search(state, pre, focus_groups, 2, select_any_cap, nullptr, options_cache, stats);

    if (!state.search.found_exact)
    {
        auto space = estimate_search_space(pre,
                                           focus_groups,
                                           state.search.flags,
                                           select_any_cap,
                                           nullptr,
                                           options_cache,
                                           stats,
                                           kConfig.focused_space_cap);
        int limit = (space <= static_cast<uint64_t>(kConfig.focused_node_limit))
                        ? 0
                        : kConfig.focused_node_limit;
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

    if (!state.search.found_exact)
    {
        std::unordered_set<int> exact_focus_groups(focus_groups.begin(), focus_groups.end());
        logger.log(std::format("[solver] Focused exact fallback: {} groups", focus_groups.size()));

        state.search.selections = state.best.best.selections;
        state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);
        auto exact_space = estimate_search_space(pre,
                                                 focus_groups,
                                                 state.search.flags,
                                                 select_any_cap,
                                                 &exact_focus_groups,
                                                 options_cache,
                                                 stats,
                                                 kConfig.exact_focused_space_cap);
        int exact_limit = (exact_space <= static_cast<uint64_t>(kConfig.exact_focused_node_limit))
                              ? 0
                              : kConfig.exact_focused_node_limit;
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

    logger.log(
        std::format("[solver] After focused search: exact={}, missing={}, extra={}, size_mm={}, "
                    "hash_mm={}",
                    state.search.found_exact,
                    state.best.best.missing,
                    state.best.best.extra,
                    state.best.best.size_mismatch,
                    state.best.best.hash_mismatch));
}

// Phase 5: Global fallback -- widening passes with increasing caps.
void run_global_fallback(
    SolverState& state,
    const Precompute& pre,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats)
{
    auto& logger = Logger::instance();

    if (state.best.has_best)
        state.search.selections = state.best.best.selections;
    state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);

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
        if (state.search.found_exact || state.progress.deadline_exceeded)
            return {};

        if (state.best.has_best)
            state.search.selections = state.best.best.selections;
        state.search.flags = rebuild_flags(*pre.installer, state.search.selections, pre.overrides);

        options_cache.clear();
        auto search_space = estimate_search_space(pre,
                                                  global_order,
                                                  state.search.flags,
                                                  cap,
                                                  exact_groups,
                                                  options_cache,
                                                  stats,
                                                  kConfig.global_space_cap);
        int node_limit = (search_space <= static_cast<uint64_t>(kConfig.global_node_limit))
                             ? 0
                             : kConfig.global_node_limit;
        if (cap == kSelectAnyCapFull)
        {
            int full_limit = kConfig.full_pass_default_limit;
            if (state.best.has_best && (state.best.best.missing > 0 || state.best.best.extra > 0))
                full_limit = kConfig.full_pass_imperfect_limit;
            if (node_limit == 0)
                node_limit = full_limit;
            else
                node_limit = std::min(node_limit, full_limit);
        }

        int capped_before = stats.capped_select_any_options;
        int pruned_before = stats.pruned_node_limit;
        run_backtrack_pass(
            state, pre, global_order, node_limit, label, cap, exact_groups, options_cache, stats);
        return {
            stats.pruned_node_limit > pruned_before,
            stats.capped_select_any_options - capped_before,
        };
    };

    auto narrow = run_global_pass(kSelectAnyCapNarrow, "global");
    auto medium = GlobalPassOutcome{};
    if (!state.search.found_exact)
    {
        logger.log(std::format("[solver] Option widening: SelectAny cap {} -> {}",
                               format_option_cap(kSelectAnyCapNarrow),
                               format_option_cap(kSelectAnyCapMedium)));
        medium = run_global_pass(kSelectAnyCapMedium, "global-widened");
    }

    bool capped_any = (narrow.capped_options > 0 || medium.capped_options > 0);
    bool unresolved_after_medium =
        !state.search.found_exact && state.best.has_best &&
        (state.best.best.missing > 0 || state.best.best.extra > 0 ||
         state.best.best.size_mismatch > 0 || state.best.best.hash_mismatch > 0);
    bool need_full_fallback =
        !state.search.found_exact && (medium.hit_limit || capped_any || unresolved_after_medium);
    if (need_full_fallback)
    {
        if (!state.search.found_exact && state.best.has_best)
        {
            auto sim_best = simulate(
                *pre.installer, *pre.atoms, state.best.best.selections, nullptr, pre.overrides);
            auto mismatched = collect_mismatched_dests(sim_best, *pre.target, *pre.excluded);
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
            !state.search.found_exact && state.best.has_best &&
            (state.best.best.missing > 0 || state.best.best.extra > 0 ||
             state.best.best.size_mismatch > 0 || state.best.best.hash_mismatch > 0);
        if (unresolved_after_targeted)
        {
            logger.log(std::format("[solver] Option widening: SelectAny cap {} -> {}",
                                   format_option_cap(kSelectAnyCapMedium),
                                   format_option_cap(kSelectAnyCapFull)));
            run_global_pass(kSelectAnyCapFull, "global-full");
        }
    }
}

}  // namespace mo2core

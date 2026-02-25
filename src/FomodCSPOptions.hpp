#pragma once

#include "FomodCSPTypes.h"

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

/**
 * @brief Free functions that enumerate solver-relevant plugin options
 *        for a given (step, group, flag-state, cap) tuple.
 *
 * Used by the CSP solver to materialize the candidate option set for
 * each group during backtracking, with caching keyed on the tuple
 * defined here so that repeated visits to the same (group, flag
 * signature) pair do not recompute the option list.
 */

namespace mo2core
{

/**
 * @brief Check if a group requires exact (exhaustive) search mode.
 *
 * @param gidx         Group index to check.
 * @param exact_groups Optional set of group indices designated for exhaustive search.
 *                     Pass `nullptr` when no groups are in exact mode.
 * @return `true` if @p gidx is present in @p exact_groups.
 */
bool is_exact_group_mode(int gidx, const std::unordered_set<int>* exact_groups);

/**
 * @brief Compute the effective SelectAny cap for a group.
 * Returns kSelectAnyCapFull when the group is in exact mode, otherwise the given cap.
 *
 * @param gidx           Group index.
 * @param select_any_cap Default SelectAny cap to use when the group is not in exact mode.
 * @param exact_groups   Optional set of group indices designated for exhaustive search.
 * @return `kSelectAnyCapFull` (0, uncapped) if the group is in exact mode,
 *         otherwise @p select_any_cap unchanged.
 */
int effective_select_any_cap(int gidx,
                             int select_any_cap,
                             const std::unordered_set<int>* exact_groups);

/**
 * @brief Return a human-readable "StepName / GroupName" label for logging.
 *
 * @param pre Precomputed solver data (used to access installer step/group names).
 * @param g   The group reference to format.
 * @return A string of the form `step N "StepName" / group M "GroupName"`.
 */
std::string group_name(const Precompute& pre, const GroupRef& g);

/**
 * @brief Enumerate valid selection options for a group, returning cached results when available.
 * Returns a vector of GroupOption bitmasks representing valid plugin selections for the group,
 * ordered by evidence score. Results are cached by (group index, flag signature, cap, exact mode)
 * so repeated calls with the same effective state return in O(1).
 *
 * @param gidx          Group index into `Precompute::groups`.
 * @param pre           Precomputed solver data.
 * @param flags         Current condition-flag values (used for type evaluation and cache keying).
 * @param select_any_cap Maximum SelectAny option cap.
 * @param exact_groups  Optional set of group indices to run in exhaustive mode.
 * @param cache         Option cache (populated on miss, read on hit).
 * @param stats         Diagnostic statistics accumulator (tracks cache hits, pruning, etc.).
 * @return A const reference to the cached options and their precomputed profiles.
 */
const CachedOptions& get_options_for_group(
    int gidx,
    const Precompute& pre,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats);

}  // namespace mo2core

#pragma once

#include "FomodCSPTypes.h"

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/// @brief Check if a group requires exact (exhaustive) search mode.
bool is_exact_group_mode(int gidx, const std::unordered_set<int>* exact_groups);

/// @brief Compute the effective SelectAny cap for a group.
/// Returns kSelectAnyCapFull when the group is in exact mode, otherwise the given cap.
int effective_select_any_cap(int gidx,
                             int select_any_cap,
                             const std::unordered_set<int>* exact_groups);

/// @brief Return a human-readable "StepName / GroupName" label for logging.
std::string group_name(const Precompute& pre, const GroupRef& g);

/// @brief Enumerate valid selection options for a group, returning cached results when available.
/// Returns a vector of GroupOption bitmasks representing valid plugin selections for the group,
/// ordered by evidence score. Results are cached by (group index, flag signature, cap, exact mode)
/// so repeated calls with the same effective state return in O(1).
const CachedOptions& get_options_for_group(
    int gidx,
    const Precompute& pre,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats);

}  // namespace mo2core

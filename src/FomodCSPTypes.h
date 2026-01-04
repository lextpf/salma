#pragma once

#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodDependencyEvaluator.h"
#include "FomodIR.h"
#include "FomodPropagator.h"
#include "Types.h"

#include <chrono>
#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/**
 * @brief Reproduction quality metrics comparing a simulated install to the target file tree.
 *
 * Counts are computed per destination file. Lower counts in every error category
 * means a better reproduction; `exact()` returns true when no errors remain.
 */
struct ReproMetrics
{
    int missing = 0;        ///< Target files not produced by the simulation.
    int extra = 0;          ///< Simulated files absent from the target.
    int size_mismatch = 0;  ///< Files present in both but with differing sizes.
    int hash_mismatch = 0;  ///< Files matching in size but differing in content hash.
    int reproduced = 0;     ///< Files successfully reproduced (size and hash match).

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

/**
 * @brief Reference to a plugin group within the FOMOD installer, with flat plugin offsets.
 *
 * Groups are addressed by their (step, group) index pair. `flat_start` gives the
 * offset into the global flat plugin array so per-plugin precomputed data (evidence,
 * unique support, etc.) can be looked up without re-walking the step hierarchy.
 */
struct GroupRef
{
    int step_idx = 0;      ///< Index of the containing step in `FomodInstaller::steps`.
    int group_idx = 0;     ///< Index of the group within the step.
    int flat_start = 0;    ///< Offset of the first plugin in the global flat plugin array.
    int plugin_count = 0;  ///< Number of plugins in this group.
};

using GroupOption = std::vector<bool>;

/**
 * @brief Mutable search state carried through backtracking and local search.
 *
 * Holds the current plugin selections, condition-flag map, and search progress
 * counters. Modified in place as the solver explores and backtracks.
 */
struct SolverSearchState
{
    std::vector<std::vector<std::vector<bool>>>
        selections;  ///< Current per-step/group/plugin selections.
    std::unordered_map<std::string, std::string> flags;  ///< Current FOMOD condition-flag values.
    int nodes_explored = 0;    ///< Total search nodes visited across all passes.
    bool found_exact = false;  ///< True when an exact (zero-error) reproduction has been found.
};

/**
 * @brief Tracks the best solution found so far during the solve.
 *
 * Updated whenever a candidate with better `ReproMetrics` is discovered.
 */
struct SolverBestResult
{
    SolverResult best;          ///< The best selection set and its error counts.
    ReproMetrics best_metrics;  ///< Metrics corresponding to `best`.
    bool has_best = false;      ///< False until the first candidate is recorded.
};

/**
 * @brief Progress tracking and deadline enforcement for the solver.
 *
 * Rate-limits log output to one progress line per `PROGRESS_NODE_INTERVAL`
 * nodes or `PROGRESS_TIME_INTERVAL_MS` milliseconds, whichever comes first.
 * Also enforces an overall time deadline; once exceeded, search phases abort.
 */
struct SolverProgress
{
    int last_progress_nodes = 0;  ///< Node count at the last progress report.
    std::chrono::steady_clock::time_point last_progress_time = std::chrono::steady_clock::now();
    static constexpr int PROGRESS_NODE_INTERVAL = 1'000;  ///< Min nodes between log lines.
    static constexpr int PROGRESS_TIME_INTERVAL_MS =
        1'000;  ///< Min milliseconds between log lines.

    int64_t estimated_total = 0;  ///< Estimated total nodes for the current pass (0 = unknown).
    std::chrono::steady_clock::time_point pass_start_time = std::chrono::steady_clock::now();
    int64_t pass_start_nodes = 0;  ///< Node count at the start of the current pass.

    std::chrono::steady_clock::time_point
        deadline{};                  ///< Hard wall-clock deadline for the entire solve.
    bool deadline_exceeded = false;  ///< Set to true once `deadline` is passed.
};

/**
 * @brief Composite solver state bundling search state, best result, and progress.
 *
 * Passed by reference through all solver phases so that greedy, local search,
 * backtracking, and repair passes share a single consistent state.
 */
struct SolverState
{
    SolverSearchState search;  ///< Current mutable search state.
    SolverBestResult best;     ///< Best solution found so far.
    SolverProgress progress;   ///< Progress tracking and deadline.
};

/**
 * @brief Diagnostic statistics for option pruning and search behavior.
 *
 * Accumulated across all solver phases. Counters in the first group track
 * option-generation reductions; the second group tracks search-tree pruning.
 */
struct SolverStats
{
    /// @name Option-generation counters
    /// @{
    int dropped_extra_only_options = 0;  ///< Options dropped because they only produce extra files.
    int collapsed_equivalent_options = 0;  ///< Options collapsed due to identical destination sets.
    int forced_unique_options = 0;         ///< Groups forced to a single option (unique evidence).
    int capped_select_any_options = 0;     ///< Options trimmed by the SelectAny cap.
    /// @}

    /// @name Search-tree pruning counters
    /// @{
    int pruned_extra_only = 0;   ///< Subtrees pruned because all options are extra-only.
    int pruned_lower_bound = 0;  ///< Subtrees pruned by lower-bound comparison against best.
    int pruned_memo = 0;         ///< Subtrees pruned by memoization hit.
    int skipped_invisible = 0;   ///< Groups skipped because their step is not visible.
    int pruned_node_limit = 0;   ///< Subtrees abandoned after exceeding the per-pass node limit.
    /// @}

    std::vector<bool> logged_group_options;  ///< Tracks which groups have had their options logged
                                             ///< (avoids duplicate log output).
};

/**
 * @brief Precomputed profile of a single group selection option.
 *
 * Captures the effect that selecting a particular combination of plugins in a
 * group would have: which destinations it produces, how much evidence supports
 * it, and which condition flags it writes. Used for pruning and ordering during
 * option enumeration.
 */
struct OptionProfile
{
    GroupOption option;      ///< Boolean mask of selected plugins in this group.
    int evidence_score = 0;  ///< Sum of per-plugin evidence scores for selected plugins.
    int unique_support = 0;  ///< Count of target destinations uniquely supplied by this option.
    int useful_dests = 0;    ///< Destinations produced that exist in the target.
    int extra_dests = 0;     ///< Destinations produced that are not in the target.
    bool sets_needed_flag =
        false;  ///< True if this option sets a flag required by a step/condition.
    std::unordered_set<std::string> produced;  ///< Destination paths produced.
    std::unordered_set<std::string>
        produced_atoms;  ///< Atom keys produced (source-level identifiers).
    std::unordered_map<std::string, std::string>
        flags_written;  ///< Condition flags set by the selected plugins.

    OptionProfile() = default;
    OptionProfile(OptionProfile&&) noexcept = default;
    OptionProfile& operator=(OptionProfile&&) noexcept = default;
};

/**
 * @brief Cached group options and their precomputed profiles.
 *
 * Stored in an `OptionCacheKey`-keyed map so that option enumeration for the
 * same group under identical flag state is not repeated.
 */
struct CachedOptions
{
    std::vector<GroupOption> options;     ///< Valid selection combinations for this group.
    std::vector<OptionProfile> profiles;  ///< Corresponding profile for each option.
};

/**
 * @brief Central precomputed data structure for the CSP solver.
 *
 * Built once before the solve begins and shared (read-only) by all solver
 * phases. Captures the flattened group list, per-plugin evidence, flag
 * dependency graphs, destination-to-group reverse indices, and the
 * independent-component decomposition used by the backtracker.
 */
struct Precompute
{
    /// @name Input references (non-owning)
    /// @{
    const FomodInstaller* installer = nullptr;
    const ExpandedAtoms* atoms = nullptr;
    const AtomIndex* atom_index = nullptr;
    const TargetTree* target = nullptr;
    const std::unordered_set<std::string>* excluded = nullptr;
    const InferenceOverrides* overrides = nullptr;
    const PropagationResult* propagation = nullptr;
    /// @}

    /// @name Flattened group list and per-plugin evidence
    /// @{
    std::vector<GroupRef> groups;  ///< All groups across all steps, in installer order.
    std::vector<int> evidence;  ///< Per-flat-plugin evidence score (higher = more likely needed).
    /// @}

    /// @name Per-plugin reverse indices
    /// @{
    std::vector<int> plugin_to_group;  ///< Maps flat plugin index to its group index.
    std::vector<int>
        plugin_unique_support;  ///< Count of target dests uniquely supplied by this plugin.
    /// @}

    /// @name Flag dependency graph
    /// @{
    std::unordered_set<std::string>
        needed_flags;  ///< All flags read by step-visibility or plugin-type conditions.
    std::vector<std::unordered_set<std::string>>
        group_sets_flags;  ///< Per-group: flags written by plugins.
    std::vector<std::unordered_set<std::string>>
        group_reads_flags;  ///< Per-group: flags read by conditions.
    std::vector<std::vector<std::string>>
        group_cache_flags;  ///< Per-group: sorted flag keys for cache hashing.
    std::unordered_map<std::string, std::vector<int>>
        flag_to_setter_groups;  ///< Maps flag name to groups that can set it.
    std::vector<std::string>
        memo_flags;  ///< Sorted union of needed and written flags, used for memoization signatures.
    std::vector<std::unordered_set<std::string>>
        group_dests;  ///< Per-group: destination paths any plugin in the group can produce.
    /// @}

    /// @name Destination reverse indices (for lower-bound pruning and repair)
    /// @{
    std::unordered_map<std::string, std::vector<int>>
        dest_to_groups;  ///< Groups that can produce each destination.
    std::unordered_map<std::string, std::vector<int>>
        dest_to_plugins;  ///< Flat plugin indices that can produce each destination.
    std::unordered_map<std::string, std::vector<int>>
        dest_to_size_match_groups;  ///< Groups that can produce a size-matching file for the
                                    ///< destination.
    std::unordered_map<std::string, std::vector<int>>
        dest_to_hash_capable_groups;  ///< Groups that can produce a hash-matching file for the
                                      ///< destination.
    /// @}

    /// @name Contested destinations and component decomposition
    /// @{
    std::unordered_set<std::string>
        conditional_dests;  ///< Destinations whose production depends on flags or conditions.
    std::vector<int> contested_plugins;  ///< Flat plugin indices that appear in multiple
                                         ///< destination-conflict sets.
    std::vector<std::vector<int>>
        components;  ///< Independent group components (groups sharing no destinations or flags),
                     ///< for parallel backtracking.
    /// @}

    Precompute() = default;
    Precompute(Precompute&&) noexcept = default;
    Precompute& operator=(Precompute&&) noexcept = default;
};

/**
 * @brief Cache key for option computations, identifying a group under a specific flag state.
 *
 * Two invocations with the same key produce identical option lists, so the
 * result can be looked up from the `CachedOptions` cache instead of regenerated.
 */
struct OptionCacheKey
{
    int group_idx = 0;        ///< Index into `Precompute::groups`.
    uint64_t flags_sig = 0;   ///< Hash of the flag values relevant to this group's conditions.
    int select_any_cap = 0;   ///< Maximum number of SelectAny options to enumerate.
    bool exact_mode = false;  ///< When true, uncapped enumeration is used for this group.

    bool operator==(const OptionCacheKey& rhs) const
    {
        return group_idx == rhs.group_idx && flags_sig == rhs.flags_sig &&
               select_any_cap == rhs.select_any_cap && exact_mode == rhs.exact_mode;
    }
};

/// @brief Hash functor for `OptionCacheKey`.
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

/**
 * @brief Memoization key for subtree pruning during backtracking.
 *
 * Captures the remaining search position, the flag state, and the
 * contested-plugin selection signature. If a subtree rooted at the same key
 * has already been explored with an equal-or-better result, it is skipped.
 */
struct MemoKey
{
    int next_idx = 0;             ///< Next group index in the search order.
    uint64_t flag_state_sig = 0;  ///< Hash of current condition-flag values.
    uint64_t contested_sig = 0;   ///< Hash of current selections for contested plugins.

    bool operator==(const MemoKey& rhs) const
    {
        return next_idx == rhs.next_idx && flag_state_sig == rhs.flag_state_sig &&
               contested_sig == rhs.contested_sig;
    }
};

/// @brief Hash functor for `MemoKey`.
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

/**
 * @brief Undo record for a single flag change during backtracking.
 *
 * Stored on a stack so that flag mutations can be reversed when the solver
 * unwinds a search frame.
 */
struct FlagDelta
{
    std::string name;        ///< Flag that was modified.
    bool had_value = false;  ///< True if the flag existed before the change.
    std::string old_value;   ///< Previous value (meaningful only when `had_value` is true).
};

/**
 * @brief Configuration for a single backtracking pass over a set of groups.
 *
 * Specifies the group visitation order, a per-pass node budget, the
 * memoization table, and whether incremental flag tracking is enabled
 * (O(N) flag updates instead of full rebuilds at each leaf).
 */
struct SearchPlan
{
    std::vector<int> order;  ///< Group indices in the order they will be explored.
    std::vector<int>
        order_pos;  ///< Inverse map: `order_pos[group_idx]` = position in `order` (-1 if absent).
    int node_limit = 0;  ///< Maximum nodes to explore in this pass (0 = unlimited).
    std::unordered_map<MemoKey, ReproMetrics, MemoKeyHash> memo;  ///< Subtree memoization table.
    bool incremental_flags =
        false;  ///< When true, flags are updated incrementally rather than rebuilt from scratch.
};

/**
 * @brief Internal constants controlling the maximum number of enumerated
 *        options for SelectAny (combinatorial) groups.
 *
 * Lower caps speed up early solver phases; later phases widen or remove the
 * cap to explore the full option space if needed.
 */
namespace csp_detail
{
static constexpr int kSelectAnyCapNarrow = 64;   ///< Tight cap for initial greedy/local search.
static constexpr int kSelectAnyCapMedium = 256;  ///< Medium cap for component and repair phases.
static constexpr int kSelectAnyCapFull = 0;      ///< No cap (0 = enumerate all valid combinations).
}  // namespace csp_detail

// Re-export for internal use.
using csp_detail::kSelectAnyCapFull;
using csp_detail::kSelectAnyCapMedium;
using csp_detail::kSelectAnyCapNarrow;

}  // namespace mo2core

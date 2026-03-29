#pragma once

#include "FomodCSPTypes.h"
#include "FomodIR.h"

#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

struct SimulatedTree;

// ---------------------------------------------------------------------------
// Solver tuning constants
// ---------------------------------------------------------------------------

/**
 * @brief Tuning constants that control time budgets, search space caps, and
 *        node limits for each phase of the FOMOD CSP solver.
 */
struct SolverConfig
{
    // -- Time / checkpoint limits --

    /** @brief Maximum wall-clock time for the entire solve. (seconds) */
    int time_limit_seconds = 600;
    /** @brief Maximum number of state checkpoints kept during backtracking. */
    size_t max_checkpoints = 4096;

    // -- Per-phase search space caps (estimated combinations) and node limits --

    /** @brief Search space estimate cap for the greedy phase. (combinations) */
    uint64_t greedy_space_cap = 1'000'000'000ULL;

    /** @brief Search space estimate cap per component in Phase 2. (combinations) */
    uint64_t component_space_cap = 10'000'000ULL;
    /** @brief Maximum nodes explored per component backtrack in Phase 2. (node count) */
    int component_node_limit = 2'000'000;

    /** @brief Maximum nodes explored during residual repair in Phase 3. (node count) */
    int residual_node_limit = 3'000'000;

    /** @brief Search space estimate cap for mismatch-focused search in Phase 4. (combinations) */
    uint64_t focused_space_cap = 10'000'000ULL;
    /** @brief Maximum nodes explored during focused backtrack in Phase 4. (node count) */
    int focused_node_limit = 6'000'000;

    /** @brief Search space estimate cap for exact-match focused fallback in Phase 4. (combinations) */
    uint64_t exact_focused_space_cap = 12'000'000ULL;
    /** @brief Maximum nodes explored during exact-match focused backtrack. (node count) */
    int exact_focused_node_limit = 8'000'000;

    /** @brief Search space estimate cap for global fallback passes in Phase 5. (combinations) */
    uint64_t global_space_cap = 10'000'000ULL;
    /** @brief Maximum nodes explored per global backtrack pass in Phase 5. (node count) */
    int global_node_limit = 10'000'000;

    // -- Full pass limits (Phase 5 with uncapped SelectAny) --

    /**
     * @brief Node limit for full-width passes when the current best is already perfect. (node
     * count)
     */
    int full_pass_default_limit = 2'000'000;
    /** @brief Node limit for full-width passes when mismatches remain. (node count) */
    int full_pass_imperfect_limit = 6'000'000;
};

/** @brief Global default solver configuration instance. */
extern const SolverConfig kConfig;

// ---------------------------------------------------------------------------
// Helper functions shared between FomodCSPSolver.cpp and FomodCSPSolverPhases.cpp
// ---------------------------------------------------------------------------

/**
 * @brief Reconstruct the condition-flag map by replaying selected plugins up
 *        to a given (step, group) pair. When @p stop_step_idx is negative the
 *        entire installer is replayed.
 *
 * @param installer   The FOMOD installer whose steps/groups/plugins define the flags.
 * @param selections  Current plugin selections grid `[step][group][plugin]`.
 * @param overrides   Optional external condition overrides for step visibility.
 * @param stop_step_idx Step index at which to stop replaying (-1 = process all steps).
 * @param stop_group_idx Group index within the stop step at which to stop (-1 = all groups).
 * @return An unordered map of flag name to flag value, representing the accumulated
 *         condition-flag state up to the specified position.
 */
std::unordered_map<std::string, std::string> rebuild_flags(
    const FomodInstaller& installer,
    const std::vector<std::vector<std::vector<bool>>>& selections,
    const InferenceOverrides* overrides,
    int stop_step_idx = -1,
    int stop_group_idx = -1);

/**
 * @brief Check whether a FOMOD step is visible given the current flag state
 *        and any external visibility overrides.
 *
 * @param step      The FOMOD step to evaluate.
 * @param step_idx  Index of the step in `FomodInstaller::steps` (used to look up overrides).
 * @param flags     Current condition-flag values accumulated from prior selections.
 * @param overrides Optional external condition overrides for step visibility.
 * @return `true` if the step is visible (has no visibility condition or the condition is met).
 */
bool step_visible_with_flags(const FomodStep& step,
                             size_t step_idx,
                             const std::unordered_map<std::string, std::string>& flags,
                             const InferenceOverrides* overrides);

/**
 * @brief Simulate the current selections and score them against the target
 *        tree. Updates the solver best-solution state when an improvement is
 *        found and emits periodic progress logs.
 *
 * @param state     Mutable solver state (search state, best result, and progress counters).
 * @param installer The FOMOD installer definition.
 * @param atoms     Expanded file-install atoms.
 * @param target    Target file tree to compare against.
 * @param excluded  Destination paths excluded from scoring.
 * @param overrides Optional external condition overrides.
 * @return The `ReproMetrics` for the current selection (missing, extra, size/hash mismatches).
 */
ReproMetrics evaluate_candidate(SolverState& state,
                                const FomodInstaller& installer,
                                const ExpandedAtoms& atoms,
                                const TargetTree& target,
                                const std::unordered_set<std::string>& excluded,
                                const InferenceOverrides* overrides);

/**
 * @brief Collect destination paths that differ between a simulated install
 *        tree and the target tree (missing, wrong size, or wrong hash).
 *
 * @param sim      The simulated install tree produced by the current selections.
 * @param target   The target file tree to compare against.
 * @param excluded Destination paths excluded from comparison.
 * @return A sorted vector of destination paths where the simulation disagrees
 *         with the target (missing, extra, size mismatch, or hash mismatch).
 */
std::vector<std::string> collect_mismatched_dests(const SimulatedTree& sim,
                                                  const TargetTree& target,
                                                  const std::unordered_set<std::string>& excluded);

/**
 * @brief Identify which solver groups contain file-install atoms that touch
 *        the given mismatched destination paths.
 *
 * @param pre        Precomputed solver data (destination-to-group reverse indices, flag graph).
 * @param mismatched Sorted list of mismatched destination paths.
 * @return A sorted vector of group indices whose plugins can produce files at
 *         the mismatched destinations, including transitive flag-dependency prerequisites.
 */
std::vector<int> groups_for_mismatches(const Precompute& pre,
                                       const std::vector<std::string>& mismatched);

/**
 * @brief Run a local search restricted to groups that affect mismatched
 *        destinations, attempting to resolve remaining file discrepancies.
 *
 * @param state         Mutable solver state (modified in place).
 * @param pre           Precomputed solver data.
 * @param repair_groups Group indices that affect the mismatched destinations.
 * @param mismatched    Destination paths where the current best differs from the target.
 */
void targeted_repair_search(SolverState& state,
                            const Precompute& pre,
                            const std::vector<int>& repair_groups,
                            const std::vector<std::string>& mismatched);

/**
 * @brief Format a large integer with SI suffixes (K, M, B) for log output.
 *
 * @param n The integer to format.
 * @return A human-readable string (e.g. "1.5M", "320K", "42").
 */
std::string format_count(int64_t n);

/**
 * @brief Format a SelectAny option cap value as a human-readable label for
 *        log output (e.g. "narrow", "medium", "full").
 *
 * @param select_any_cap The cap value (64 = narrow, 256 = medium, 0 = full/uncapped).
 * @return A label string such as `"narrow (64)"`, `"medium (256)"`, or `"full"`.
 */
std::string format_option_cap(int select_any_cap);

/**
 * @brief Greedy forward pass: iterate groups in order and pick the
 *        best-scoring option combination for each group independently.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param select_any_cap Maximum number of SelectAny options to enumerate per group.
 * @param exact_groups  Optional set of group indices to run in exhaustive (uncapped) mode.
 * @param cache        Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void greedy_solve(SolverState& state,
                  const Precompute& pre,
                  int select_any_cap,
                  const std::unordered_set<int>* exact_groups,
                  std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                  SolverStats& stats);

/**
 * @brief Iterative improvement: repeatedly re-solve each group in @p order,
 *        keeping changes that improve the score, for up to @p max_passes.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param order        Group indices specifying the visitation order.
 * @param max_passes   Maximum number of full improvement passes before stopping.
 * @param select_any_cap Maximum number of SelectAny options to enumerate per group.
 * @param exact_groups  Optional set of group indices to run in exhaustive mode.
 * @param cache        Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void local_search(SolverState& state,
                  const Precompute& pre,
                  const std::vector<int>& order,
                  int max_passes,
                  int select_any_cap,
                  const std::unordered_set<int>* exact_groups,
                  std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                  SolverStats& stats);

/**
 * @brief Estimate the total number of candidate combinations for the given
 *        group ordering under current flags and caps. Returns early once the
 *        estimate exceeds @p limit.
 *
 * @param pre          Precomputed solver data.
 * @param order        Group indices to include in the estimate.
 * @param flags        Current condition-flag values.
 * @param select_any_cap Maximum SelectAny option cap.
 * @param exact_groups  Optional set of group indices to run in exhaustive mode.
 * @param cache        Option cache (may be populated as a side effect).
 * @param stats        Diagnostic statistics accumulator.
 * @param limit        Early-exit threshold; once the product exceeds this, the function returns.
 * @return The estimated combination count, or a value greater than @p limit if it was exceeded.
 */
uint64_t estimate_search_space(
    const Precompute& pre,
    const std::vector<int>& order,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats,
    uint64_t limit);

/**
 * @brief Systematic backtracking search over the given group ordering with
 *        branch-and-bound pruning. Stops after @p node_limit evaluations
 *        (0 = unlimited). @p label identifies the pass in log output.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param order        Group indices specifying the visitation order.
 * @param node_limit   Maximum nodes to explore before aborting (0 = unlimited).
 * @param label        Label for log output identifying this pass (e.g. "global", "focused").
 * @param select_any_cap Maximum SelectAny option cap.
 * @param exact_groups  Optional set of group indices to run in exhaustive mode.
 * @param cache        Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void run_backtrack_pass(
    SolverState& state,
    const Precompute& pre,
    const std::vector<int>& order,
    int node_limit,
    const std::string& label,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats);

// ---------------------------------------------------------------------------
// Phase functions (implemented in FomodCSPSolverPhases.cpp)
// ---------------------------------------------------------------------------
//
/**
 * The solver pipeline runs phases in sequence, short-circuiting on exact match:
 *
 * ```mermaid
 * flowchart TD
 *     A[run_initial_phases] --> B{exact match?}
 *     B -->|yes| Z[return]
 *     B -->|no| C[run_component_decomposition]
 *     C --> D{exact match?}
 *     D -->|yes| Z
 *     D -->|no| E[run_residual_repair]
 *     E --> F{exact match?}
 *     F -->|yes| Z
 *     F -->|no| G[run_focused_search]
 *     G --> H{exact match?}
 *     H -->|yes| Z
 *     H -->|no| I[run_global_fallback]
 *     I --> Z
 * ```
 */

/**
 * @brief Phase 1: Run a greedy forward pass, iterative local search, and a
 *        first targeted repair on groups that affect remaining mismatches.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param select_any_cap Maximum SelectAny option cap for this phase.
 * @param options_cache Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void run_initial_phases(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/**
 * @brief Phase 2: Decompose the problem into independent components and run
 *        per-component local search followed by backtracking within each.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data (must have `components` populated).
 * @param select_any_cap Maximum SelectAny option cap for this phase.
 * @param options_cache Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void run_component_decomposition(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/**
 * @brief Phase 3: Near-perfect residual repair when the best solution has no
 *        missing or extra files but still has a small number of size/hash
 *        mismatches (m=0, e=0).
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param select_any_cap Maximum SelectAny option cap for this phase.
 * @param options_cache Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void run_residual_repair(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/**
 * @brief Phase 4: Mismatch-focused search restricting local search and
 *        backtracking to groups that affect currently mismatched destinations,
 *        with a second exact-match fallback pass.
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param select_any_cap Maximum SelectAny option cap for this phase.
 * @param options_cache Option cache shared across solver phases.
 * @param stats        Diagnostic statistics accumulator.
 */
void run_focused_search(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/**
 * @brief Phase 5: Global fallback running widening backtrack passes over all
 *        groups with progressively larger SelectAny option caps (narrow,
 *        medium, full).
 *
 * @param state        Mutable solver state (modified in place).
 * @param pre          Precomputed solver data.
 * @param options_cache Option cache shared across solver phases (cleared between cap changes).
 * @param stats        Diagnostic statistics accumulator.
 */
void run_global_fallback(
    SolverState& state,
    const Precompute& pre,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

}  // namespace mo2core

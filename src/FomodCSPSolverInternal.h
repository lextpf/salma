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

/// @brief Tuning constants that control time budgets, search space caps, and
///        node limits for each phase of the FOMOD CSP solver.
struct SolverConfig
{
    // -- Time / checkpoint limits --

    /// @brief Maximum wall-clock time for the entire solve. (seconds)
    int time_limit_seconds = 600;
    /// @brief Maximum number of state checkpoints kept during backtracking.
    size_t max_checkpoints = 4096;

    // -- Per-phase search space caps (estimated combinations) and node limits --

    /// @brief Search space estimate cap for the greedy phase. (combinations)
    uint64_t greedy_space_cap = 1'000'000'000ULL;

    /// @brief Search space estimate cap per component in Phase 2. (combinations)
    uint64_t component_space_cap = 10'000'000ULL;
    /// @brief Maximum nodes explored per component backtrack in Phase 2. (node count)
    int component_node_limit = 2'000'000;

    /// @brief Maximum nodes explored during residual repair in Phase 3. (node count)
    int residual_node_limit = 3'000'000;

    /// @brief Search space estimate cap for mismatch-focused search in Phase 4. (combinations)
    uint64_t focused_space_cap = 10'000'000ULL;
    /// @brief Maximum nodes explored during focused backtrack in Phase 4. (node count)
    int focused_node_limit = 6'000'000;

    /// @brief Search space estimate cap for exact-match focused fallback in Phase 4. (combinations)
    uint64_t exact_focused_space_cap = 12'000'000ULL;
    /// @brief Maximum nodes explored during exact-match focused backtrack. (node count)
    int exact_focused_node_limit = 8'000'000;

    /// @brief Search space estimate cap for global fallback passes in Phase 5. (combinations)
    uint64_t global_space_cap = 10'000'000ULL;
    /// @brief Maximum nodes explored per global backtrack pass in Phase 5. (node count)
    int global_node_limit = 10'000'000;

    // -- Full pass limits (Phase 5 with uncapped SelectAny) --

    /// @brief Node limit for full-width passes when the current best is already perfect. (node
    /// count)
    int full_pass_default_limit = 2'000'000;
    /// @brief Node limit for full-width passes when mismatches remain. (node count)
    int full_pass_imperfect_limit = 6'000'000;
};

/// @brief Global default solver configuration instance.
extern const SolverConfig kConfig;

// ---------------------------------------------------------------------------
// Helper functions shared between FomodCSPSolver.cpp and FomodCSPSolverPhases.cpp
// ---------------------------------------------------------------------------

/// @brief Reconstruct the condition-flag map by replaying selected plugins up
///        to a given (step, group) pair. When @p stop_step_idx is negative the
///        entire installer is replayed.
std::unordered_map<std::string, std::string> rebuild_flags(
    const FomodInstaller& installer,
    const std::vector<std::vector<std::vector<bool>>>& selections,
    const InferenceOverrides* overrides,
    int stop_step_idx = -1,
    int stop_group_idx = -1);

/// @brief Check whether a FOMOD step is visible given the current flag state
///        and any external visibility overrides.
bool step_visible_with_flags(const FomodStep& step,
                             size_t step_idx,
                             const std::unordered_map<std::string, std::string>& flags,
                             const InferenceOverrides* overrides);

/// @brief Simulate the current selections and score them against the target
///        tree. Updates the solver best-solution state when an improvement is
///        found and emits periodic progress logs.
ReproMetrics evaluate_candidate(SolverState& state,
                                const FomodInstaller& installer,
                                const ExpandedAtoms& atoms,
                                const TargetTree& target,
                                const std::unordered_set<std::string>& excluded,
                                const InferenceOverrides* overrides);

/// @brief Collect destination paths that differ between a simulated install
///        tree and the target tree (missing, wrong size, or wrong hash).
std::vector<std::string> collect_mismatched_dests(const SimulatedTree& sim,
                                                  const TargetTree& target,
                                                  const std::unordered_set<std::string>& excluded);

/// @brief Identify which solver groups contain file-install atoms that touch
///        the given mismatched destination paths.
std::vector<int> groups_for_mismatches(const Precompute& pre,
                                       const std::vector<std::string>& mismatched);

/// @brief Run a local search restricted to groups that affect mismatched
///        destinations, attempting to resolve remaining file discrepancies.
void targeted_repair_search(SolverState& state,
                            const Precompute& pre,
                            const std::vector<int>& repair_groups,
                            const std::vector<std::string>& mismatched);

/// @brief Format a large integer with SI suffixes (K, M, B) for log output.
std::string format_count(int64_t n);

/// @brief Format a SelectAny option cap value as a human-readable label for
///        log output (e.g. "narrow", "medium", "full").
std::string format_option_cap(int select_any_cap);

/// @brief Greedy forward pass: iterate groups in order and pick the
///        best-scoring option combination for each group independently.
void greedy_solve(SolverState& state,
                  const Precompute& pre,
                  int select_any_cap,
                  const std::unordered_set<int>* exact_groups,
                  std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                  SolverStats& stats);

/// @brief Iterative improvement: repeatedly re-solve each group in @p order,
///        keeping changes that improve the score, for up to @p max_passes.
void local_search(SolverState& state,
                  const Precompute& pre,
                  const std::vector<int>& order,
                  int max_passes,
                  int select_any_cap,
                  const std::unordered_set<int>* exact_groups,
                  std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
                  SolverStats& stats);

/// @brief Estimate the total number of candidate combinations for the given
///        group ordering under current flags and caps. Returns early once the
///        estimate exceeds @p limit.
uint64_t estimate_search_space(
    const Precompute& pre,
    const std::vector<int>& order,
    const std::unordered_map<std::string, std::string>& flags,
    int select_any_cap,
    const std::unordered_set<int>* exact_groups,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& cache,
    SolverStats& stats,
    uint64_t limit);

/// @brief Systematic backtracking search over the given group ordering with
///        branch-and-bound pruning. Stops after @p node_limit evaluations
///        (0 = unlimited). @p label identifies the pass in log output.
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

/// @brief Phase 1: Run a greedy forward pass, iterative local search, and a
///        first targeted repair on groups that affect remaining mismatches.
void run_initial_phases(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/// @brief Phase 2: Decompose the problem into independent components and run
///        per-component local search followed by backtracking within each.
void run_component_decomposition(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/// @brief Phase 3: Near-perfect residual repair when the best solution has no
///        missing or extra files but still has a small number of size/hash
///        mismatches (m=0, e=0).
void run_residual_repair(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/// @brief Phase 4: Mismatch-focused search restricting local search and
///        backtracking to groups that affect currently mismatched destinations,
///        with a second exact-match fallback pass.
void run_focused_search(
    SolverState& state,
    const Precompute& pre,
    int select_any_cap,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

/// @brief Phase 5: Global fallback running widening backtrack passes over all
///        groups with progressively larger SelectAny option caps (narrow,
///        medium, full).
void run_global_fallback(
    SolverState& state,
    const Precompute& pre,
    std::unordered_map<OptionCacheKey, CachedOptions, OptionCacheKeyHash>& options_cache,
    SolverStats& stats);

}  // namespace mo2core

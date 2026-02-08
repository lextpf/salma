#pragma once

#include "Export.h"
#include "FomodAtom.h"
#include "FomodDependencyEvaluator.h"
#include "FomodIR.h"
#include "Types.h"

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/**
 * @struct SolverResult
 * @brief Output of the CSP solver: plugin selections and match quality metrics.
 * @author Alex (https://github.com/lextpf)
 *
 * `selections` is a 3-D boolean grid indexed `[step][group][plugin]` that
 * records which plugins the solver chose. The quality counters (`missing`,
 * `extra`, `size_mismatch`, `hash_mismatch`) describe how closely the
 * resulting install plan reproduces the target file tree.
 */
struct SolverResult
{
    std::vector<std::vector<std::vector<bool>>>
        selections; /**< [step][group][plugin] selection grid */
    std::unordered_map<std::string, std::string>
        inferred_flags;       /**< Flags set by the chosen plugins */
    bool exact_match = false; /**< True iff the install reproduces the target tree exactly */
    int nodes_explored = 0;   /**< CSP search-tree node count (diagnostics) */
    int missing = 0;          /**< Target dests not produced by selections */
    int extra = 0;            /**< Selections that produce dests not in target */
    int size_mismatch = 0;    /**< Dests produced with wrong uncompressed size */
    int hash_mismatch = 0;    /**< Dests produced with wrong content hash */
};

/**
 * @struct InferenceOverrides
 * @brief Tri-state overrides for external dependencies the solver cannot evaluate.
 * @author Alex (https://github.com/lextpf)
 *
 * Some FOMOD conditions depend on external state (e.g. game version, other
 * installed mods) that is unavailable during inference. These overrides let
 * the caller force individual conditional patterns or step visibility
 * conditions to true, false, or unknown so the solver can prune or explore
 * branches accordingly.
 *
 * Each element is an `ExternalConditionOverride` tri-state:
 * - `Unknown` -- the solver treats the condition as ambiguous and explores
 *   both branches (default when no external context is available).
 * - `ForceTrue` -- the condition is assumed to hold.
 * - `ForceFalse` -- the condition is assumed to not hold.
 */
struct InferenceOverrides
{
    /**
     * Per conditional pattern: tri-state external dependency override.
     * Indexed by the conditional-pattern index in `FomodInstaller::conditional_patterns`.
     */
    std::vector<ExternalConditionOverride> conditional_active;
    /**
     * Per step visibility condition: tri-state external dependency override.
     * Indexed by the step index in `FomodInstaller::steps`.
     */
    std::vector<ExternalConditionOverride> step_visible;
};

struct PropagationResult;

/**
 * @brief Infer FOMOD plugin selections that best reproduce a target file tree.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * Explores the selection space as a constraint-satisfaction problem, pruning
 * branches via atom/target set comparison. Returns the best-scoring selection
 * found, or an exact match if one exists. Optional `overrides` pin external
 * dependency results, and `propagation` supplies pre-computed constraint
 * propagation data to accelerate the search.
 *
 * @param installer  Parsed FOMOD installer containing steps, groups, and plugins.
 * @param atoms      Expanded file-install atoms indexed per-plugin and per-conditional.
 * @param atom_index Reverse index mapping destination paths to the atoms that produce them.
 * @param target     Target file tree the solver tries to reproduce (dest -> size/hash).
 * @param excluded_dests Destination paths to ignore during scoring (e.g. metadata files).
 * @param overrides  Optional tri-state overrides for external conditions the solver
 *                   cannot evaluate. Pass `nullptr` to leave all external conditions as Unknown.
 * @param propagation Optional pre-computed constraint propagation result. When non-null,
 *                    narrowed domains are used to prune the option space before search.
 * @return A `SolverResult` containing the best plugin selections found, inferred flag
 *         values, match quality metrics (`missing`, `extra`, `size_mismatch`,
 *         `hash_mismatch`), and `exact_match` indicating a perfect reproduction.
 * @pre `installer` must contain at least one step with at least one group.
 *
 * @note The internal phase pipeline (greedy -> propagate ->
 *       component-decompose -> residual repair -> focused search ->
 *       global fallback) and its tuning constants are declared in
 *       `FomodCSPSolverInternal.h`. The Mermaid flowchart in that
 *       header (above the per-phase `run_*` declarations) shows the
 *       short-circuit transitions on exact match.
 * @see FomodCSPSolverInternal.h, mo2core::kConfig
 */
MO2_API SolverResult solve_fomod_csp(const FomodInstaller& installer,
                                     const ExpandedAtoms& atoms,
                                     const AtomIndex& atom_index,
                                     const TargetTree& target,
                                     const std::unordered_set<std::string>& excluded_dests,
                                     const InferenceOverrides* overrides = nullptr,
                                     const PropagationResult* propagation = nullptr);

}  // namespace mo2core

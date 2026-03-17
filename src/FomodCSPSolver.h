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
    std::vector<std::vector<std::vector<bool>>> selections;  // [step][group][plugin]
    std::unordered_map<std::string, std::string> inferred_flags;
    bool exact_match = false;
    int nodes_explored = 0;
    int missing = 0;
    int extra = 0;
    int size_mismatch = 0;
    int hash_mismatch = 0;
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
 */
struct InferenceOverrides
{
    // Per conditional pattern: tri-state external dependency override.
    std::vector<ExternalConditionOverride> conditional_active;
    // Per step visibility condition: tri-state external dependency override.
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
 */
MO2_API SolverResult solve_fomod_csp(const FomodInstaller& installer,
                                     const ExpandedAtoms& atoms,
                                     const AtomIndex& atom_index,
                                     const TargetTree& target,
                                     const std::unordered_set<std::string>& excluded_dests,
                                     const InferenceOverrides* overrides = nullptr,
                                     const PropagationResult* propagation = nullptr);

}  // namespace mo2core

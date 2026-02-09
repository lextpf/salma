#pragma once

#include "Export.h"
#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodDependencyEvaluator.h"
#include "FomodIR.h"
#include "Types.h"

#include <string>
#include <tuple>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/**
 * @struct PropagationResult
 * @brief Output of the constraint propagation pre-pass.
 * @author Alex (https://github.com/lextpf)
 *
 * Contains narrowed plugin domains and a list of groups that were fully
 * resolved without backtracking. When `fully_resolved` is true the CSP
 * solver can be skipped entirely.
 */
struct PropagationResult
{
    std::vector<std::vector<std::vector<bool>>>
        narrowed_domains; /**< Per-group remaining candidate selections after propagation.
                             [step][group][plugin] = usable */
    std::vector<std::tuple<int, int>>
        resolved_groups;         /**< Groups fully resolved by propagation (step_idx, group_idx). */
    bool fully_resolved = false; /**< Whether all groups were resolved (no CSP needed). */
};

/**
 * @brief Deterministic constraint propagation pre-pass for FOMOD inference.
 * @ingroup FomodService
 *
 * Walks the installer steps in document order and narrows each group's
 * plugin domain using three strategies:
 *
 * 1. **Plugin type constraints** - `Required` and `NotUsable` plugins
 *    are pinned; `SelectAll` groups are resolved immediately.
 * 2. **File evidence** - plugins whose files are entirely absent from
 *    the target tree are eliminated; unique file matches pin a plugin.
 * 3. **Cardinality** - after elimination, groups with only one valid
 *    combination under their `FomodGroupType` are resolved.
 *
 * All steps are treated as visible because original step visibility at
 * install time cannot be determined during inference.
 *
 * The propagator iterates a domain-narrowing operator \f$\mathcal{T}\f$
 * until fixpoint:
 *
 * \f[ D^{(k+1)} = \mathcal{T}(D^{(k)}), \quad k < 16 \f]
 *
 * terminating at the smallest \f$k \le 16\f$ with
 * \f$D^{(k)} = D^{(k-1)}\f$ (see `MAX_ITERATIONS` in
 * `FomodPropagator.cpp`).
 *
 * The operator \f$\mathcal{T}\f$ is the composition of the three
 * narrowing rules listed above:
 *
 * \f[ \mathcal{T} = \mathcal{C} \circ \mathcal{F} \circ \mathcal{P} \f]
 *
 * where, applied to each group's domain:
 * - \f$\mathcal{P}\f$ pins `Required` plugins, eliminates `NotUsable`
 *   plugins, and resolves `SelectAll` groups (plugin-type rule);
 * - \f$\mathcal{F}\f$ eliminates plugins whose files are entirely
 *   absent from the target tree and pins plugins that uniquely
 *   produce a target file (file-evidence rule);
 * - \f$\mathcal{C}\f$ resolves a group whose remaining plugin set
 *   admits exactly one valid combination under the group's
 *   `FomodGroupType` (cardinality rule).
 *
 * Each \f$\mathcal{T}\f$ application is monotone: a plugin removed
 * from a domain is never re-added. The fixpoint therefore always
 * exists; the iteration cap is a guard against malformed installers,
 * not against non-termination.
 *
 * ```mermaid
 * flowchart TD
 *     start[Initialize all domains to true] --> loop{Changed & iter < 16?}
 *     loop -->|yes| step1[Apply plugin type constraints]
 *     step1 --> step2[Filter by file evidence]
 *     step2 --> step3[Resolve cardinality]
 *     step3 --> loop
 *     loop -->|no| done[Return PropagationResult]
 * ```
 *
 * @param installer      Parsed FOMOD installer definition.
 * @param atoms          Expanded file-install atoms indexed per-plugin and per-conditional.
 * @param atom_index     Reverse index mapping destination paths to atoms.
 * @param target         Target file tree the solver tries to reproduce.
 * @param excluded_dests Destination paths to ignore during evidence evaluation.
 * @param overrides      Tri-state overrides for external conditions.
 * @param context        Optional external dependency context for plugin type evaluation.
 *                       Pass `nullptr` during standalone inference.
 * @return A `PropagationResult` containing narrowed plugin domains, a list of
 *         fully resolved groups, and a `fully_resolved` flag indicating whether
 *         all groups were determined without needing the CSP solver.
 */
MO2_API PropagationResult propagate(const FomodInstaller& installer,
                                    const ExpandedAtoms& atoms,
                                    const AtomIndex& atom_index,
                                    const TargetTree& target,
                                    const std::unordered_set<std::string>& excluded_dests,
                                    const InferenceOverrides& overrides,
                                    const FomodDependencyContext* context);

}  // namespace mo2core

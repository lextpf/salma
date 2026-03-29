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
 *
 * Contains narrowed plugin domains and a list of groups that were fully
 * resolved without backtracking. When `fully_resolved` is true the CSP
 * solver can be skipped entirely.
 */
struct PropagationResult
{
    // Per-group: remaining candidate selections after propagation.
    // [step][group][plugin] = usable
    std::vector<std::vector<std::vector<bool>>> narrowed_domains;
    // Groups fully resolved by propagation (step_idx, group_idx).
    std::vector<std::tuple<int, int>> resolved_groups;
    // Whether all groups were resolved (no CSP needed).
    bool fully_resolved = false;
};

/**
 * @brief Deterministic constraint propagation pre-pass for FOMOD inference.
 * @ingroup FomodService
 *
 * Walks the installer steps in document order and narrows each group's
 * plugin domain using three strategies:
 *
 * 1. **Plugin type constraints** - `Required` and `NotUsable` plugins are
 *    pinned; `SelectAll`
 * groups are resolved immediately.
 * 2. **File evidence** - plugins whose files are entirely absent from the
 *    target tree are
 * eliminated; unique file matches pin a plugin.
 * 3. **Cardinality** - after elimination, groups with only one valid
 *    combination under their
 * `FomodGroupType` are resolved.
 *
 * All steps are treated as visible because original step visibility at
 * install time cannot be determined during inference.
 *
 * The propagator runs a fixpoint iteration loop:
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

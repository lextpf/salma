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
 */
MO2_API PropagationResult propagate(const FomodInstaller& installer,
                                    const ExpandedAtoms& atoms,
                                    const AtomIndex& atom_index,
                                    const TargetTree& target,
                                    const std::unordered_set<std::string>& excluded_dests,
                                    const InferenceOverrides& overrides,
                                    const FomodDependencyContext* context);

}  // namespace mo2core

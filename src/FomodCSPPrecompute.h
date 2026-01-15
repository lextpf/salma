#pragma once

#include "FomodCSPTypes.h"

#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/**
 * @brief Recursively collect all flag names referenced by a condition tree.
 *
 * @param c   The condition tree root to walk.
 * @param out Output set; flag names found in `Flag`-type nodes are inserted here.
 */
void collect_condition_flags(const FomodCondition& c, std::unordered_set<std::string>& out);

/**
 * @brief Check if a condition depends on external state (file, plugin, or game dependencies).
 * Flag and composite-of-flags conditions return false; all other types return true.
 *
 * @param c The condition to inspect.
 * @return `true` if any leaf in the condition tree is a File, Plugin, Game, Fomod, Fomm,
 *         or Fose condition; `false` if the tree consists only of Flag and Composite nodes.
 */
bool condition_depends_on_external_state(const FomodCondition& c);

/**
 * @brief Combine two hash values using Boost-style hash combine.
 *
 * @param seed In/out hash accumulator. Modified in place.
 * @param v    The value to mix into @p seed.
 */
void hash_combine(uint64_t& seed, uint64_t v);

/**
 * @brief Hash a subset of flag values for use as a cache key.
 * Only the flags named in @p keys are included in the hash.
 *
 * @param flags The full condition-flag map.
 * @param keys  Sorted list of flag names to include in the hash.
 * @return A 64-bit hash combining only the flag values for the specified keys.
 */
uint64_t hash_flag_subset(const std::unordered_map<std::string, std::string>& flags,
                          const std::vector<std::string>& keys);

/**
 * @brief Compute per-plugin evidence scores based on file overlap with the target tree.
 * Scores are higher for unique file matches and hash-confirmed matches.
 *
 * For each target destination, evidence is accumulated per-plugin as:
 *
 * $$E_{\text{plugin}} = \sum_{\text{dest}} w(\text{plugin}, \text{dest})$$
 *
 * where the per-(plugin, destination) weight $w$ is:
 * - $+3$ when the plugin is the **unique** size-compatible producer of that destination,
 * - $+2$ when multiple plugins can produce it but this plugin has a **hash match**,
 * - $+1$ when multiple plugins can produce it and no hash confirmation is available.
 *
 * Additionally, flag-setter evidence is propagated from conditional patterns and
 * step-visibility conditions: when a conditional pattern produces $h$ target-matching
 * files, each plugin that sets the required flag value receives $+h$ evidence.
 *
 * @param installer The FOMOD installer definition.
 * @param atoms     Expanded file-install atoms.
 * @param atom_index Reverse index mapping destination paths to atoms.
 * @param target    Target file tree (dest -> size/hash).
 * @param excluded  Destination paths excluded from evidence computation.
 * @return A flat vector of evidence scores indexed by flat plugin index.
 */
std::vector<int> compute_evidence(const FomodInstaller& installer,
                                  const ExpandedAtoms& atoms,
                                  const AtomIndex& atom_index,
                                  const TargetTree& target,
                                  const std::unordered_set<std::string>& excluded);

/**
 * @brief Build the Precompute data structure from installer, atoms, target tree, and overrides.
 * This is the main entry point for precomputation; the result is used throughout the solver.
 *
 * @param installer   The FOMOD installer definition.
 * @param atoms       Expanded file-install atoms.
 * @param atom_index  Reverse index mapping destination paths to atoms.
 * @param target      Target file tree (dest -> size/hash).
 * @param excluded    Destination paths excluded from scoring.
 * @param overrides   Optional external condition overrides (may be `nullptr`).
 * @param propagation Optional constraint propagation result (may be `nullptr`).
 * @param groups      Flattened group references (moved into the result).
 * @param evidence    Flat per-plugin evidence scores (moved into the result).
 * @return A fully populated `Precompute` struct containing reverse indices,
 *         flag dependency graphs, component decomposition, and all other
 *         precomputed data needed by the solver.
 */
Precompute build_precompute(const FomodInstaller& installer,
                            const ExpandedAtoms& atoms,
                            const AtomIndex& atom_index,
                            const TargetTree& target,
                            const std::unordered_set<std::string>& excluded,
                            const InferenceOverrides* overrides,
                            const PropagationResult* propagation,
                            std::vector<GroupRef> groups,
                            std::vector<int> evidence);

}  // namespace mo2core

#pragma once

#include "FomodCSPTypes.h"

#include <cstdint>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/// @brief Recursively collect all flag names referenced by a condition tree.
void collect_condition_flags(const FomodCondition& c, std::unordered_set<std::string>& out);

/// @brief Check if a condition depends on external state (file, plugin, or game dependencies).
/// Flag and composite-of-flags conditions return false; all other types return true.
bool condition_depends_on_external_state(const FomodCondition& c);

/// @brief Combine two hash values using Boost-style hash combine.
void hash_combine(uint64_t& seed, uint64_t v);

/// @brief Hash a subset of flag values for use as a cache key.
/// Only the flags named in @p keys are included in the hash.
uint64_t hash_flag_subset(const std::unordered_map<std::string, std::string>& flags,
                          const std::vector<std::string>& keys);

/// @brief Compute per-plugin evidence scores based on file overlap with the target tree.
/// Scores are higher for unique file matches and hash-confirmed matches.
std::vector<int> compute_evidence(const FomodInstaller& installer,
                                  const ExpandedAtoms& atoms,
                                  const AtomIndex& atom_index,
                                  const TargetTree& target,
                                  const std::unordered_set<std::string>& excluded);

/// @brief Build the Precompute data structure from installer, atoms, target tree, and overrides.
/// This is the main entry point for precomputation; the result is used throughout the solver.
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

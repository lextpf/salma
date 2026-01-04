#pragma once

#include "Export.h"
#include "FomodAtom.h"
#include "FomodIR.h"
#include "Types.h"

#include <string>
#include <unordered_map>
#include <vector>

namespace mo2core
{

struct InferenceOverrides;

/**
 * @struct SimulatedTree
 * @brief The file tree produced by a simulated FOMOD installation.
 *
 * Maps each lowercased destination path to the single winning FomodAtom
 * after priority and document-order conflict resolution. Used by the CSP
 * solver to compare a candidate selection against the real target tree
 * without performing any actual extraction.
 */
struct SimulatedTree
{
    std::unordered_map<std::string, FomodAtom> files;  // dest -> winning atom

    SimulatedTree() = default;
    SimulatedTree(SimulatedTree&&) noexcept = default;
    SimulatedTree& operator=(SimulatedTree&&) noexcept = default;
};

/**
 * @brief Run a forward simulation of a FOMOD installation for a given selection.
 * @ingroup FomodService
 *
 * Replicates the priority and document-order conflict resolution semantics of
 * FomodService::execute_file_operations, but produces an in-memory file tree
 * instead of writing to disk. Conditional install patterns are evaluated when
 * a `context` and/or `overrides` are provided; otherwise they are skipped.
 */
MO2_API SimulatedTree simulate(const FomodInstaller& installer,
                               const ExpandedAtoms& atoms,
                               const std::vector<std::vector<std::vector<bool>>>& selections,
                               const FomodDependencyContext* context = nullptr,
                               const InferenceOverrides* overrides = nullptr);

/// In-place version of simulate() that reuses an existing SimulatedTree's
/// allocation. The tree's file map is cleared before simulation begins,
/// but the underlying hash-map capacity is preserved across calls.
MO2_API void simulate_into(SimulatedTree& tree,
                           const FomodInstaller& installer,
                           const ExpandedAtoms& atoms,
                           const std::vector<std::vector<std::vector<bool>>>& selections,
                           const FomodDependencyContext* context = nullptr,
                           const InferenceOverrides* overrides = nullptr);

}  // namespace mo2core

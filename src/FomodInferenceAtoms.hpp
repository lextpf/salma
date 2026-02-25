#pragma once

#include "Export.h"
#include "FomodAtom.h"
#include "FomodCSPSolver.h"
#include "FomodIR.h"
#include "InferenceDiagnostics.h"
#include "Logger.h"
#include "Utils.h"

#include <nlohmann/json.hpp>

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace mo2core
{

/**
 * @brief Inference-side wrapper for path-traversal validation.
 *
 * Thin forwarder to `mo2core::is_safe_destination` so call sites in
 * the inference pipeline read symmetrically with the FomodService
 * pipeline. Both paths share the same rule set.
 *
 * @param dest  Normalized mod-relative destination path to validate.
 * @return `true` if the path is safe to use as an install destination.
 * @throw Does not throw.
 * @see mo2core::is_safe_destination (Utils.h) for the canonical rules.
 */
bool is_safe_dest(const std::string& dest);

/**
 * @brief Expand a single FomodFileEntry into concrete FomodAtom objects by
 *        matching against archive entries.
 *
 * For folder entries, performs a prefix search over @p sorted_entries to find
 * all archive members under the source directory, producing one atom per
 * match. For single-file entries, produces exactly one atom. Unsafe
 * destinations (path traversal) are logged and skipped.
 *
 * @param entry          The FOMOD file/folder mapping to expand.
 * @param sorted_entries Lexicographically sorted list of all archive entry paths,
 *                       used for efficient prefix-based folder expansion.
 * @param entry_sizes    Map from archive entry path to file size in bytes,
 *                       used to populate FomodAtom::file_size.
 * @param doc_order      Document-order index for conflict resolution; higher
 *                       values win ties when two atoms share the same priority.
 * @param origin         Whether this entry comes from required files, a plugin,
 *                       or a conditional install pattern.
 * @param plugin_idx     Flat plugin index (across all steps/groups), or -1 if
 *                       the entry is not from a plugin.
 * @param cond_idx       Conditional pattern index, or -1 if the entry is not
 *                       from a conditional install pattern.
 * @param out            Output vector to which expanded atoms are appended.
 * @throw std::bad_alloc if memory allocation for atoms fails.
 */
void expand_entry(const FomodFileEntry& entry,
                  const std::vector<std::string>& sorted_entries,
                  const std::unordered_map<std::string, uint64_t>& entry_sizes,
                  int doc_order,
                  FomodAtom::Origin origin,
                  int plugin_idx,
                  int cond_idx,
                  std::vector<FomodAtom>& out);

/**
 * @brief Expand all FOMOD file entries into atoms using three ordered passes.
 *
 * Pass 1 expands required files (lowest document_order range), pass 2 expands
 * normal plugin files then always-install/installIfUsable plugin files (middle
 * range), and pass 3 expands conditional install patterns (highest range).
 * The separated passes preserve document_order invariants required by the
 * FOMOD spec for correct conflict resolution.
 *
 * @return An ExpandedAtoms struct with atoms grouped by origin (required,
 *         per-plugin, per-conditional).
 * @throw std::bad_alloc if memory allocation for atoms fails.
 */
ExpandedAtoms expand_all_atoms(const FomodInstaller& installer,
                               const std::vector<std::string>& sorted_entries,
                               const std::unordered_map<std::string, uint64_t>& entry_sizes);

/**
 * @brief Build an index that groups all atoms by their destination path.
 *
 * Iterates every atom in @p atoms (required, plugin, and conditional) and
 * collects them into an AtomIndex keyed by dest_path. This index is used
 * downstream to detect destination conflicts and compute excluded destinations.
 *
 * @return An AtomIndex mapping each destination path to the list of atoms
 *         that target it.
 * @throw std::bad_alloc if memory allocation for the index fails.
 */
AtomIndex build_atom_index(const ExpandedAtoms& atoms);

/**
 * @brief Identify destinations that are only targeted by auto-install atoms
 *        and should be excluded from solver scoring.
 *
 * A destination is excluded when every atom targeting it is either an
 * always_install, install_if_usable, or required-origin atom, and all atoms
 * originate from the same source path. These destinations carry no signal for
 * differentiating plugin selections, so including them would add noise.
 * Destinations with any conditional-origin atom are never excluded, because
 * which conditionals fire depends on plugin selections.
 *
 * @return Set of destination paths to exclude from match scoring.
 * @throw std::bad_alloc if memory allocation for the set fails.
 */
std::unordered_set<std::string> compute_excluded_dests(const AtomIndex& atom_index);

/**
 * @brief Build a TargetTree from the files already installed in the mod
 *        directory.
 *
 * Creates a TargetFile entry for each installed file (keyed by relative path),
 * recording its size for later comparison against candidate atoms. MO2
 * metadata files (e.g. meta.ini) are skipped since they are never part of
 * FOMOD installations.
 *
 * @param installed_files  Map from mod-relative path to file size in bytes,
 *                         representing the current state of the target directory.
 * @return A TargetTree mapping each installed file's relative path to its
 *         TargetFile metadata.
 * @throw std::bad_alloc if memory allocation for the tree fails.
 */
TargetTree build_target_tree(const std::unordered_map<std::string, uint64_t>& installed_files);

/**
 * @brief Convert a SolverResult + InferenceDiagnostics into a JSON response.
 *
 * Walks the installer's step/group/plugin hierarchy and cross-references the
 * solver's 3-D boolean selection grid to classify each plugin as selected or
 * deselected. Out-of-bounds indices are logged as warnings and default to
 * false (deselected).
 *
 * Schema v2 wire format:
 * - Top level carries `schema_version` (2), `steps` array, and a `diagnostics`
 *   block (run-level summary).
 * - Each step is an object with `name`, `confidence`, `visible`, `reasons`,
 *   and `groups`.
 * - Each group is an object with `name`, `confidence`, `resolved_by`,
 *   `reasons`, `plugins` (selected as object array), and `deselected`
 *   (deselected as object array).
 * - Each plugin is an object with `name`, `selected`, `confidence`, and
 *   `reasons`.
 *
 * @return A JSON object matching the schema-v2 wire format.
 * @throw std::bad_alloc if JSON construction runs out of memory.
 */
MO2_API nlohmann::json assemble_json(const FomodInstaller& installer,
                                     const SolverResult& result,
                                     const InferenceDiagnostics& diagnostics);

}  // namespace mo2core

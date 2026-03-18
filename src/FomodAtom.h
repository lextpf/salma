#pragma once

#include <concepts>
#include <cstdint>
#include <string>
#include <unordered_map>
#include <vector>

namespace mo2core
{

/**
 * @class FomodAtom
 * @brief Single file-install operation produced by FOMOD XML evaluation.
 * @author Alex (https://github.com/lextpf)
 * @ingroup FomodService
 *
 * An atom is the smallest unit of work in a FOMOD installation: one source
 * file mapped to one destination path. The full install plan is assembled by
 * collecting atoms from required files, selected plugins, and conditional
 * install patterns, then resolving conflicts by priority and document order.
 *
 * ## :material-file-tree: Origin Types
 *
 * |       Origin | Source                          | Lifetime                       |
 * |--------------|---------------------------------|--------------------------------|
 * |     Required | `<requiredInstallFiles>`        | Always included                |
 * |       Plugin | `<files>` inside a `<plugin>`   | Included when plugin selected  |
 * |  Conditional | `<conditionalFileInstalls>`     | Included when pattern matches  |
 *
 * ## :material-sort-variant: Conflict Resolution
 *
 * When multiple atoms target the same destination path, the winner is chosen
 * by highest `priority` first, then highest `document_order` as a tiebreaker.
 * `content_hash` and `file_size` allow skipping redundant extractions when
 * the winning atom is byte-identical to an already-installed file.
 */
struct FomodAtom
{
    std::string source_path;  // normalized archive entry
    std::string dest_path;    // normalized destination
    int priority = 0;
    int document_order = 0;
    uint64_t content_hash = 0;  // FNV-1a of archive bytes (0 = unknown)
    uint64_t file_size = 0;

    enum class Origin
    {
        Required,
        Plugin,
        Conditional
    };
    Origin origin = Origin::Required;
    int plugin_index = -1;  // flat index across all steps/groups
    int conditional_index = -1;

    bool always_install = false;
    bool install_if_usable = false;
};

/**
 * @struct TargetFile
 * @brief Metadata for a file already present in the target directory.
 * @author Alex (https://github.com/lextpf)
 *
 * Used during conflict resolution to detect when the winning atom is
 * byte-identical to an existing file, allowing the extraction to be skipped.
 */
struct TargetFile
{
    uint64_t size = 0;
    uint64_t hash = 0;  // FNV-1a (0 = not computed)
};

/// @brief Maps a lowercased destination path to every atom that targets it.
using AtomIndex = std::unordered_map<std::string, std::vector<FomodAtom>>;

/// @brief Maps a lowercased destination path to metadata of the installed file.
using TargetTree = std::unordered_map<std::string, TargetFile>;

/**
 * @struct ExpandedAtoms
 * @brief Atoms grouped by origin, ready for selection-based filtering.
 * @author Alex (https://github.com/lextpf)
 *
 * `required` atoms are always installed. `per_plugin` and `per_conditional`
 * vectors are indexed by the corresponding flat plugin or conditional index,
 * so the caller can include or exclude each group based on user selections
 * and dependency evaluation results.
 */
struct ExpandedAtoms
{
    std::vector<FomodAtom> required;
    std::vector<std::vector<FomodAtom>> per_plugin;       // [flat_plugin_index]
    std::vector<std::vector<FomodAtom>> per_conditional;  // [conditional_index]

    /// Apply a callable to every atom across all origin containers.
    /// Deducing `this` provides const and non-const overloads from one definition.
    template <typename Self, typename Func>
        requires std::invocable<Func, decltype(*std::declval<Self>().required.begin())>
    void for_each(this Self&& self, Func&& fn)
    {
        for (auto&& a : self.required)
            fn(a);
        for (auto&& v : self.per_plugin)
            for (auto&& a : v)
                fn(a);
        for (auto&& v : self.per_conditional)
            for (auto&& a : v)
                fn(a);
    }
};

}  // namespace mo2core

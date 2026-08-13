#pragma once

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

/**
 * @brief Shared value types that describe the FOMOD install data model.
 * @author Alex (https://github.com/lextpf)
 *
 * Enums, structs, and small value types kept in one header to avoid circular
 * includes.
 *
 * ## :material-source-branch: Scope
 *
 * Install and inference run inside the engine DLL, which the server reaches
 * through `mo2server::SalmaEngine`. Do not write new C++ against the types
 * below.
 *
 * `PluginType` is the only one with a C++ consumer:
 * `mo2core::parse_plugin_type_string` in Utils.hpp, which tests/utils_test.cpp
 * exercises. `FileOpType`, `FileOperation`, `FomodDependencyContext` and
 * `InstallResult` have no C++ caller. They stay because they document the data
 * model the engine implements, and PARITY-NOTES.md resolves cross-references
 * through these names.
 *
 * ## :material-information-outline: Notes
 *
 * This block carries no group tag. doxide rejects one on a namespace
 * ("namespace cannot have ingroup, ignoring"), so every type below is tagged
 * into Core individually. Naming that command with its leading sigil in prose
 * would itself be parsed as the command, which is why it is spelled out here.
 */
namespace mo2core
{

/**
 * @enum FileOpType
 * @brief Discriminator for file vs folder copy operations.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Selects which copy a FileOperation performs.
 */
enum class FileOpType
{
    File,   ///< Single file copy.
    Folder  ///< Recursive folder copy.
};

/**
 * @enum PluginType
 * @brief FOMOD plugin type descriptor.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Maps directly to the `<type>` element values of the FOMOD ModuleConfig
 * schema. Controls whether a plugin is auto-selected, user-selectable, or
 * greyed out.
 *
 * String conversion runs through `enum_map<PluginType>` in Utils.hpp, using the
 * FOMOD spellings ("Required", "Recommended", "Optional", "NotUsable",
 * "CouldBeUsable"). The match is exact and case-sensitive; an unrecognized name
 * maps to `Optional`.
 */
enum class PluginType
{
    Required,      ///< Always installed, cannot be deselected.
    Recommended,   ///< Pre-selected, the user can deselect it.
    Optional,      ///< Not pre-selected, the user can select it.
    NotUsable,     ///< Greyed out, cannot be selected.
    CouldBeUsable  ///< Selectable, but the FOMOD installer warns the user first.
};

/**
 * @struct FileOperation
 * @brief A single queued file or folder copy operation.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * The engine collects one of these per file or folder node while it walks a
 * FOMOD, then sorts the queue and executes it. `src/fomod_service.rs` enqueues
 * the operations; its `execute_file_operations` sorts and runs them.
 *
 * ## :material-source-branch: Conflict Resolution
 *
 * Two operations can name the same destination. The queue is stably sorted by
 * ascending `priority`, then by ascending `document_order`, and executed in
 * that order, so the last operation that writes a destination is the one that
 * survives.
 *
 * | priority | document_order | outcome                                    |
 * |---------:|---------------:|--------------------------------------------|
 * |        0 |              3 | loses                                      |
 * |        0 |              7 | wins: same priority, enqueued later        |
 * |        5 |              1 | wins: higher priority beats enqueue order  |
 *
 * `document_order` is an enqueue counter, not an XML byte position. The engine
 * increments it once per queued node, so it records the order in which the walk
 * reached them.
 */
struct FileOperation
{
    FileOpType type;  ///< File or folder.
    /// Source path under the extracted archive root. Most call sites build
    /// this with `fs::path::string()`, which gives OS-native separators. Some
    /// FOMOD code paths produce forward-slash strings instead, and the copy
    /// step accepts both.
    std::string source;
    /// Destination path under the mod directory. Same convention as `source`:
    /// usually OS-native separators, sometimes forward slashes.
    std::string destination;
    /// FOMOD `priority` attribute. MO2 default: 0. Higher wins on conflict.
    int priority = 0;
    /// Enqueue counter. Tiebreaker when two operations share a `priority`.
    int document_order = 0;
};

/**
 * @struct FomodDependencyContext
 * @brief External state passed to the FOMOD dependency evaluator.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Carries the environment needed to evaluate `<fileDependency>`,
 * `<gameDependency>` and the other non-flag dependency types.
 *
 * The three set members do not share one matching convention: `installed_files`
 * and `installed_plugins` are matched after normalization, `installed_fomods`
 * is matched case-sensitively. Populate each member in the form its own comment
 * states, or lookups miss silently.
 */
struct FomodDependencyContext
{
    std::string game_path;  ///< Root path of the game installation.
    /// Files present in the mod directory, normalized to lowercase with
    /// forward slashes. Lookups normalize the queried path the same way.
    std::unordered_set<std::string> installed_files;
    /// Active game plugins (`.esp` / `.esm`), lowercase. Lookups lowercase the
    /// queried name before the test.
    std::unordered_set<std::string> installed_plugins;
    /// Previously installed FOMOD packages. Matched case-sensitively: unlike
    /// plugin names, FOMOD names are not lowercased. Store the exact spelling
    /// the FOMOD condition uses.
    std::unordered_set<std::string> installed_fomods;
    std::string game_version;  ///< Game version string for comparison.
    std::string archive_root;  ///< Extracted archive root directory.
};

/**
 * @struct InstallResult
 * @brief Outcome of a FOMOD install or replay attempt.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * The server never sees one of these. The flat C ABI reports only a string plus
 * a process-global success flag, so `mo2server::SalmaEngine::install_mod`
 * returns the installed path and throws on failure instead.
 */
struct InstallResult
{
    bool success = false;  ///< True when installation completed without error.
    std::string mod_path;  ///< Path to the installed mod directory.
    std::string error;     ///< Error message. Empty when `success` is true.
};

}  // namespace mo2core

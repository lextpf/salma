#pragma once

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

/**
 * @namespace mo2core
 * @brief Shared types used across the salma core library.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Enums, structs, and lightweight value types referenced by multiple
 * services. Kept in a single header to avoid circular includes.
 */
namespace mo2core
{

/**
 * @enum FileOpType
 * @brief Discriminator for file vs folder copy operations.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 */
enum class FileOpType
{
    File,  /**< Single file copy */
    Folder /**< Recursive folder copy */
};

/**
 * @enum PluginType
 * @brief FOMOD plugin type descriptor.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Maps directly to the `<type>` element values defined by the
 * FOMOD ModuleConfig schema. Controls whether a plugin is
 * auto-selected, user-selectable, or greyed out.
 */
enum class PluginType
{
    Required,     /**< Always installed, cannot be deselected */
    Recommended,  /**< Pre-selected but user can deselect */
    Optional,     /**< Not pre-selected, user can select */
    NotUsable,    /**< Greyed out, cannot be selected */
    CouldBeUsable /**< Selectable but the FOMOD installer warns the user before applying */
};

/**
 * @struct FileOperation
 * @brief A single queued file or folder copy operation.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Used by FomodService to collect install actions and by
 * FileOperations to sort and execute them. The `priority` field
 * controls overwrite order (higher wins); `document_order` breaks
 * ties using XML source position.
 */
struct FileOperation
{
    FileOpType type;         /**< File or folder */
    std::string source;      /**< Source path under the extracted archive root. Most callers build
                                  these via `fs::path::string()` (OS-native separators); some
                                  FOMOD code paths produce forward-slash strings that downstream
                                  copy code accepts unchanged. */
    std::string destination; /**< Destination path under the mod directory. Same convention as
                                  `source`: typically OS-native, occasionally forward-slash. */
    int priority = 0;        /**< FOMOD priority attribute (MO2 default: 0) */
    int document_order = 0;  /**< Enqueue counter (incremented per FomodService node), used as
                                priority tiebreaker. Not strictly XML byte-position. */
};

/**
 * @struct FomodDependencyContext
 * @brief External state passed to the FOMOD dependency evaluator.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * Provides the environment needed to evaluate `<fileDependency>`,
 * `<gameDependency>`, and other non-flag dependency types.
 */
struct FomodDependencyContext
{
    std::string game_path;                           /**< Root path of the game installation */
    std::unordered_set<std::string> installed_files; /**< Files present in the mod directory
                                                        (normalized: lowercase, forward-slash) */
    std::unordered_set<std::string>
        installed_plugins; /**< Active game plugins (.esp/.esm, lowercase) */
    std::unordered_set<std::string> installed_fomods; /**< Previously installed FOMOD packages */
    std::string game_version;                         /**< Game version string for comparison */
    std::string archive_root;                         /**< Extracted archive root directory */
};

/**
 * @struct InstallResult
 * @brief Outcome of a FOMOD install/replay attempt.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 */
struct InstallResult
{
    bool success = false; /**< Whether installation completed without error */
    std::string mod_path; /**< Path to the installed mod directory */
    std::string error;    /**< Error message if success is false */
};

}  // namespace mo2core

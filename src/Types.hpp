#pragma once

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

/**
 * @namespace mo2core
 * @brief Contains shared FOMOD value types and support services.
 * @author Alex (<https://github.com/lextpf>)
 *
 */
namespace mo2core
{

/**
 * @enum FileOpType
 * @brief Distinguishes file and folder copy operations.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 */
enum class FileOpType
{
    File,   ///< Single file copy.
    Folder  ///< Recursive folder copy.
};

/**
 * @enum PluginType
 * @brief Describes how a FOMOD option is presented and selected.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 * Values match the FOMOD `type` element. String conversion is case-sensitive and
 * maps unknown values to `Optional`.
 */
enum class PluginType
{
    Required,      ///< Always selected.
    Recommended,   ///< Selected by default.
    Optional,      ///< Not selected by default.
    NotUsable,     ///< Unavailable for selection.
    CouldBeUsable  ///< Available with a warning.
};

/**
 * @struct FileOperation
 * @brief Describes one queued file or folder copy.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 * Operations run in ascending `priority` and `document_order`. Later writes win
 * when several operations target the same destination.
 */
struct FileOperation
{
    FileOpType type;  ///< File or folder.
    /// Source path under the extracted archive root.
    std::string source;
    /// Destination path under the mod directory.
    std::string destination;
    /// FOMOD priority. Higher values win conflicts.
    int priority = 0;
    /// Enqueue counter used as the priority tie-breaker.
    int document_order = 0;
};

/**
 * @struct FomodDependencyContext
 * @brief Supplies external state to the FOMOD dependency evaluator.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 * File and plugin keys are lowercase. File keys use forward slashes. FOMOD keys
 * retain their original case.
 */
struct FomodDependencyContext
{
    std::string game_path;  ///< Game installation root.
    /// Lowercase file paths with forward slashes.
    std::unordered_set<std::string> installed_files;
    /// Lowercase active plugin names.
    std::unordered_set<std::string> installed_plugins;
    /// Case-sensitive installed FOMOD names.
    std::unordered_set<std::string> installed_fomods;
    std::string game_version;  ///< Game version used for comparison.
    std::string archive_root;  ///< Extracted archive root.
};

/**
 * @struct InstallResult
 * @brief Reports the outcome of a FOMOD install.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 */
struct InstallResult
{
    bool success = false;  ///< True after a complete install.
    std::string mod_path;  ///< Installed mod directory.
    std::string error;     ///< Failure message, or empty on success.
};

}  // namespace mo2core

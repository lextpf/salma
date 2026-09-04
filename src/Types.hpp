#pragma once

#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

/**
 * @namespace mo2core
 * @brief contains shared FOMOD value types and support services.
 * @author Alex (https://github.com/lextpf)
 *
 */
namespace mo2core
{

/**
 * @enum FileOpType
 * @brief distinguishes file and folder copy operations.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 */
enum class FileOpType
{
    File,   ///< single file copy.
    Folder  ///< recursive folder copy.
};

/**
 * @enum PluginType
 * @brief describes how a FOMOD option is presented and selected.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * values match the FOMOD `type` element. string conversion is case-sensitive and
 * maps unknown values to `Optional`.
 */
enum class PluginType
{
    Required,      ///< always selected.
    Recommended,   ///< selected by default.
    Optional,      ///< not selected by default.
    NotUsable,     ///< unavailable for selection.
    CouldBeUsable  ///< available with a warning.
};

/**
 * @struct FileOperation
 * @brief describes one queued file or folder copy.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * operations run in ascending `priority` and `document_order`. later writes win
 * when several operations target the same destination.
 */
struct FileOperation
{
    FileOpType type;  ///< file or folder.
    /// source path under the extracted archive root.
    std::string source;
    /// destination path under the mod directory.
    std::string destination;
    /// FOMOD priority. higher values win conflicts.
    int priority = 0;
    /// enqueue counter used as the priority tie-breaker.
    int document_order = 0;
};

/**
 * @struct FomodDependencyContext
 * @brief supplies external state to the FOMOD dependency evaluator.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * file and plugin keys are lowercase. file keys use forward slashes. FOMOD keys
 * retain their original case.
 */
struct FomodDependencyContext
{
    std::string game_path;  ///< game installation root.
    /// lowercase file paths with forward slashes.
    std::unordered_set<std::string> installed_files;
    /// lowercase active plugin names.
    std::unordered_set<std::string> installed_plugins;
    /// case-sensitive installed FOMOD names.
    std::unordered_set<std::string> installed_fomods;
    std::string game_version;  ///< game version used for comparison.
    std::string archive_root;  ///< extracted archive root.
};

/**
 * @struct InstallResult
 * @brief reports the outcome of a FOMOD install.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 */
struct InstallResult
{
    bool success = false;  ///< true after a complete install.
    std::string mod_path;  ///< installed mod directory.
    std::string error;     ///< failure message, or empty on success.
};

}  // namespace mo2core

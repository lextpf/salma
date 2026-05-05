#pragma once

#include <filesystem>
#include <vector>

namespace mo2core
{

/**
 * @class ModStructureDetector
 * @brief Detects candidate content roots inside archives.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ModStructureDetector
 *
 * Static utility used by InstallationService to identify which
 * subdirectory of an extracted archive contains the actual mod
 * content. This matters when archives have an extra wrapper folder
 * (e.g. `ModName-v1.2/meshes/...` instead of `meshes/...`).
 *
 * ## :material-folder-search: Recognition Heuristic
 *
 * A directory is considered a mod root if it contains any of
 * these well-known Skyrim mod folders:
 *
 * `SKSE` - `meshes` - `textures` - `interface` - `sound` - `scripts` - `seq` - `F4SE` - `SFSE` -
 * `OBSE` - `materials`
 *
 * ## :material-alert-circle-outline: Limitations
 *
 * - **Non-recursive**: find_main_mod_folders() only checks immediate
 *   children of the archive root (one level deep). Deeply nested
 *   wrapper folders (e.g. `Outer/Inner/meshes/`) are not detected.
 * - **False positives**: A folder named `textures` or `scripts` that
 *   contains non-mod content will be reported as a mod root. This is
 *   rare in practice since the input is always an extracted mod
 *   archive.
 *
 * ## :material-microsoft-windows: Platform Assumptions
 *
 * Folder name matching is **case-insensitive** (Windows filesystem
 * semantics). On case-sensitive filesystems this would need
 * normalization, but all target platforms are Windows.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * auto candidates = ModStructureDetector::find_main_mod_folders(archive_root);
 * if (candidates.size() == 1) {
 *     FileOperations::copy_directory_contents(candidates[0], mod_path);
 * }
 * ```
 *
 * @see InstallationService
 */
class ModStructureDetector
{
public:
    /**
     * @brief Check whether a directory looks like a mod root.
     *
     * @param dir Directory to inspect.
     * @return `true` if any recognized mod subfolder exists.
     * @throw std::filesystem::filesystem_error if the directory cannot
     *        be accessed (e.g. permissions error). Propagated from
     *        `std::filesystem::exists()`.
     */
    static bool has_mod_structure(const std::filesystem::path& dir);

    /**
     * @brief Find all immediate subdirectories that look like mod roots.
     *
     * Iterates one level deep under @p archive_root and returns
     * every directory for which has_mod_structure() is true.
     *
     * @param archive_root Extracted archive directory to scan.
     * @return Paths to candidate mod folders (may be empty).
     *         Returns empty on filesystem iteration errors (caught
     *         internally and logged).
     */
    static std::vector<std::filesystem::path> find_main_mod_folders(
        const std::filesystem::path& archive_root);
};

}  // namespace mo2core

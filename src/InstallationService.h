#pragma once

#include <filesystem>
#include <string>
#include "Export.h"
#include "Types.h"

namespace mo2core
{

/**
 * @class InstallationService
 * @brief FOMOD install/replay orchestrator with a simple non-FOMOD copy fallback.
 * @author Alex
 * (https://github.com/lextpf)
 * @ingroup InstallationService
 *
 * Top-level entry point called by the C API. Handles the full lifecycle
 * of a FOMOD processing
 * request: extract the archive to a temp directory,
 * detect whether a FOMOD installer is present,
 * and delegate to either the
 * FOMOD pipeline or a simple file-copy fallback.
 *
 * ## :material-swap-horizontal: Install Flow
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart LR
 *     classDef step fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
 *     classDef check fill:#134e3a,stroke:#10b981,color:#e2e8f0
 *     classDef out fill:transparent,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
 *
 *     A[extract archive]:::step
 *     B{FOMOD folder?}:::check
 *     C[FomodService pipeline]:::step
 *     D[non-FOMOD copy]:::step
 *     E[cleanup temp dir]:::step
 *
 *     A --> B
 *     B -->|yes| C
 *     B -->|no| D
 *     C --> E
 *     D --> E
 * ```
 *
 * ## :material-folder-search-outline: Non-FOMOD Detection
 *
 * When no `fomod/ModuleConfig.xml` is found, the service uses
 * ModStructureDetector to look for
 * recognizable content roots
 * (e.g. containing `meshes/`, `textures/`, `SKSE/`). If multiple
 * candidates exist, a `moduleName` from the JSON config disambiguates.
 * As a final fallback, the entire archive root is copied flat.
 *
 * ## :material-shield-check: moduleName Sanitization
 *
 * When a JSON config provides `moduleName` for non-FOMOD disambiguation,
 * the value is rejected (cleared and logged as a warning, not thrown)
 * if it contains:
 *
 * - Path separators (`/` or `\`)
 * - Parent-directory segments (`..`)
 * - A stem matching a Windows reserved device name (case-insensitive):
 *   CON, PRN, AUX, NUL, COM1-9, LPT1-9. The check ignores extension,
 *   so `con.txt` is rejected just like `con`.
 *
 * A rejected `moduleName` falls through to the "ambiguous folder"
 * fatal path if multiple candidates exist.
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * InstallationService service;
 * std::string result = service.install_mod(
 *     "C:/downloads/mod.7z",
 *     "C:/mods/MyMod",
 *     "C:/config/selections.json"
 * );
 * ```
 *
 * ## :material-alert-circle-outline: Error Model
 *
 * **Fatal errors (throw `std::runtime_error` or subclass):**
 * - Missing archive file (before extraction begins).
 * - Extraction failure (bit7z or libarchive errors propagated as-is).
 * - Invalid JSON in selections file (`nlohmann::json::parse_error`).
 * - Fatal XML parse failure in FOMOD ModuleConfig.xml.
 * - Unmet module-level dependencies (FOMOD `<moduleDependencies>`).
 * - Ambiguous non-FOMOD folder selection: multiple mod folders
 *   detected with no `moduleName` in JSON, or `moduleName` matches
 *   zero or multiple folders.
 *
 * **Tolerated errors (logged, installation continues):**
 * - Individual file copy failures during FOMOD or non-FOMOD install.
 * - FOMOD group-type validation warnings (wrong selection count).
 * - Missing plugin nodes in XML (XPath lookup failures).
 *
 * **Non-FOMOD branch:** When mod structure detection finds a single
 * candidate or none (flat-copy fallback), no additional throws occur
 * beyond the fatal cases above. Multiple candidates without a
 * disambiguating `moduleName` are fatal (see above).
 *
 * ## :material-broom: Temp Directory Cleanup
 *
 * `install_mod` creates a unique temp directory under
 * `fs::temp_directory_path()` (e.g. `fomod-<8-hex>/`) for the
 * extracted archive. The cleanup is symmetric across both exit paths:
 *
 * - On success, the temp directory is removed via `fs::remove_all`
 *   inside the success branch. Removal failures are logged as
 *   warnings but do not turn a successful install into a failure.
 * - On any thrown exception, a `catch(...)` block runs `remove_all`
 *   before re-throwing, so a partially-extracted archive does not
 *   leak into the OS temp directory. Cleanup failures inside the
 *   error path are also logged as warnings.
 *
 * The mod destination directory itself (`mod_path`) is **not**
 * cleaned up on failure -- once `fs::create_directories(mod_path)`
 * has run, partial files may already have been copied there by the
 * FOMOD pipeline. Callers that need transactional installs should
 * either install into a staging directory and atomically rename, or
 * accept that a failed install can leave half-installed content in
 * `mod_path` for the user (or MO2) to clean up.
 *
 * ## :material-help: Thread Safety
 *
 * Instances are **not** thread-safe. The C API creates a fresh
 * instance per call.
 *
 * @see CApi, ArchiveService, FomodService, ModStructureDetector
 */
class MO2_API InstallationService
{
public:
    /**
     * @brief Process a FOMOD archive into a destination directory.
     *
     * Extracts the archive to a temporary directory, detects the
     * install method (FOMOD or
     * flat copy), performs the FOMOD replay
     * or fallback copy into @p mod_path, and cleans
     * up.
     *
     * @param archive_path Path to the FOMOD-capable archive.
     * @param mod_path Destination mod
     * directory (created if missing).
     * @param json_path Optional path to a FOMOD selections JSON file.
     *        If empty, the service looks for a `.json` file next to the
     *        archive. Without a JSON config, FOMOD installs process
     *        only required files and conditional patterns - optional
     *        steps are skipped because no selections exist.
     * @return The destination mod path on success.
     * @throw std::runtime_error if the archive
     * is missing or installation fails fatally.
     */
    std::string install_mod(const std::string& archive_path,
                            const std::string& mod_path,
                            const std::string& json_path = "");

private:
    std::filesystem::path find_fomod_folder(const std::filesystem::path& archive_root);
    std::string handle_non_fomod_install(const std::filesystem::path& archive_root,
                                         const std::string& mod_path,
                                         const std::string& archive_path,
                                         const std::string& json_path);
    std::string handle_fomod_install(const std::filesystem::path& fomod_folder,
                                     const std::filesystem::path& archive_root,
                                     const std::string& mod_path,
                                     const std::string& archive_path,
                                     const std::string& temp_dir,
                                     const std::string& json_path);

    /**
     * Resolve the JSON selections path: use json_path if non-empty,
     * otherwise derive from the archive stem. Validates with is_inside.
     * Returns the resolved path or empty string if invalid/missing.
     */
    std::string resolve_json_path(const std::string& json_path, const std::string& archive_path);
};

}  // namespace mo2core

#pragma once

#include <filesystem>
#include <string>

namespace mo2server
{

/**
 * @class SalmaEngine
 * @brief Adapts the Rust engine DLL to C++ values and failures.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Server
 *
 * The DLL is loaded once and remains loaded. Returned engine strings are copied
 * and released before control returns to the caller.
 *
 * ### :material-alert-circle-outline: Results and failures
 *
 * Install failures raise `std::runtime_error`; other engine failures return empty
 * values.
 * An empty `json_path` enables the engine sidecar rule. Archive resolution returns
 * the first existing engine candidate.
 *
 * ### :material-transit-connection-variant: Loading and result ownership
 *
 * ```mermaid
 * flowchart TD
 *     call --> loaded{loaded?}
 *     loaded -- no --> probe[probe executable paths, then OS search]
 *     probe --> bind{all exports found?}
 *     bind -- no --> fail[return failure]
 *     bind -- yes --> invoke[invoke engine]
 *     loaded -- yes --> invoke
 *     invoke --> copy[copy result and call freeResult]
 * ```
 *
 * ### :material-lock-outline: Thread safety
 *
 * Installs hold a process-wide lock across the call and `installSucceeded`
 * because the success flag is process-global. Inference and archive resolution
 * can run concurrently.
 *
 * ```mermaid
 * flowchart LR
 *     install --> lock[lock install mutex]
 *     concurrent[concurrent install] -->|wait| lock
 *     lock --> call[installWithConfig]
 *     call --> release[Copy result and call freeResult]
 *     release --> status[installSucceeded]
 *     status --> unlock[Unlock mutex]
 * ```
 */
class SalmaEngine
{
  public:
    /**
     * @fn static bool SalmaEngine::ensure_loaded()
     * @brief Retries incomplete loads instead of caching failure.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Search order is the executable directory, its `salma` subdirectory, then
     * the OS search path. All required exports must resolve. Failures are not
     * cached and later calls retry.
     *
     * @return `true` after a complete bind; always `false` outside Windows.
     */
    static bool ensure_loaded();

    /**
     * @fn static std::string SalmaEngine::loaded_path()
     * @brief Reports prior load state without triggering a load.
     * @author Alex (<https://github.com/lextpf>)
     *
     * This does not trigger loading. The default-search result is a diagnostic
     * label rather than a filesystem path.
     *
     * @return The location label, or empty before a successful load.
     */
    static std::string loaded_path();

    /**
     * @fn static std::string SalmaEngine::api_version()
     * @brief Uses an empty value when the DLL cannot bind.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The ABI version pointer is static and must not pass through `freeResult`.
     *
     * @return A copy of the version text, or empty when loading fails.
     */
    static std::string api_version();

    /**
     * @fn std::string install_mod(const std::string&, const std::string&, const std::string&)
     * @brief Serializes the engine call with its process-global success flag.
     * @author Alex (<https://github.com/lextpf>)
     *
     * An empty `json_path` lets the engine select an archive sidecar. Failure
     * raises `std::runtime_error` with the engine message.
     *
     * @param archive_path Source archive.
     * @param mod_path Destination mod directory.
     * @param json_path Selections file, or empty for sidecar discovery.
     * @return The installed mod path reported by the engine.
     */
    static std::string install_mod(const std::string& archive_path,
                                   const std::string& mod_path,
                                   const std::string& json_path);

    /**
     * @fn std::string infer_selections(const std::string&, const std::string&)
     * @brief Reports inference failures as an empty result.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param archive_path Source archive.
     * @param mod_path Installed mod directory.
     * @return Schema-v2 selections JSON, or empty on failure.
     */
    static std::string infer_selections(const std::string& archive_path,
                                        const std::string& mod_path);

    /**
     * @fn std::filesystem::path resolve_mod_archive(const std::string&,
     *     const std::filesystem::path&, const std::filesystem::path&)
     * @brief Delegate MO2 archive lookup to the shared engine.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The engine applies candidate precedence and existence checks. This host does not duplicate
     * those rules.
     *
     * @param installation_file The meta.ini archive value, including any stored relative path.
     * @param mod_folder Installed mod directory used for relative candidates.
     * @param mods_dir MO2 mods root used to locate sibling data directories.
     * @return The resolved archive path, or empty when loading or resolution fails.
     */
    static std::filesystem::path resolve_mod_archive(const std::string& installation_file,
                                                     const std::filesystem::path& mod_folder,
                                                     const std::filesystem::path& mods_dir);
};

}  // namespace mo2server

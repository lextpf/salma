#pragma once

#include <filesystem>
#include <string>

namespace mo2server
{

/**
 * @class SalmaEngine
 * @brief adapts the Rust engine DLL to C++ values and failures.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Server
 *
 * the DLL is loaded once and remains loaded. returned engine strings are copied
 * and released before control returns to the caller.
 *
 * ### :material-alert-circle-outline: results and failures
 *
 * install failures raise `std::runtime_error`; other engine failures return empty
 * values.
 * an empty `json_path` enables the engine sidecar rule. archive resolution returns
 * the first existing engine candidate.
 *
 * ### :material-transit-connection-variant: loading and result ownership
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
 * ### :material-lock-outline: thread safety
 *
 * installs hold a process-wide lock across the call and `installSucceeded`
 * because the success flag is process-global. inference and archive resolution
 * can run concurrently.
 *
 * ```mermaid
 * flowchart LR
 *     install --> lock[lock install mutex]
 *     concurrent[concurrent install] -->|wait| lock
 *     lock --> call[installWithConfig]
 *     call --> status[installSucceeded]
 *     status --> release[freeResult]
 *     release --> unlock[unlock mutex]
 * ```
 */
class SalmaEngine
{
  public:
    /**
     * @fn static bool SalmaEngine::ensure_loaded()
     * @brief retries incomplete loads instead of caching failure.
     * @author Alex (https://github.com/lextpf)
     *
     * search order is the executable directory, its `salma` subdirectory, then
     * the OS search path. all required exports must resolve. failures are not
     * cached and later calls retry.
     *
     * @return `true` after a complete bind; always `false` outside Windows.
     */
    static bool ensure_loaded();

    /**
     * @fn static std::string SalmaEngine::loaded_path()
     * @brief reports prior load state without triggering a load.
     * @author Alex (https://github.com/lextpf)
     *
     * this does not trigger loading. the default-search result is a diagnostic
     * label rather than a filesystem path.
     *
     * @return the location label, or empty before a successful load.
     */
    static std::string loaded_path();

    /**
     * @fn static std::string SalmaEngine::api_version()
     * @brief uses an empty value when the DLL cannot bind.
     * @author Alex (https://github.com/lextpf)
     *
     * @return the static version string, or empty when loading fails.
     */
    static std::string api_version();

    /**
     * @fn std::string install_mod(const std::string&, const std::string&, const std::string&)
     * @brief serializes the engine call with its process-global success flag.
     * @author Alex (https://github.com/lextpf)
     *
     * an empty `json_path` lets the engine select an archive sidecar. failure
     * raises `std::runtime_error` with the engine message.
     *
     * @param archive_path source archive.
     * @param mod_path destination mod directory.
     * @param json_path selections file, or empty for sidecar discovery.
     * @return the installed mod path reported by the engine.
     */
    static std::string install_mod(const std::string& archive_path,
                                   const std::string& mod_path,
                                   const std::string& json_path);

    /**
     * @fn std::string infer_selections(const std::string&, const std::string&)
     * @brief reports inference failures as an empty result.
     * @author Alex (https://github.com/lextpf)
     *
     * @param archive_path source archive.
     * @param mod_path installed mod directory.
     * @return schema-v2 selections JSON, or empty on failure.
     */
    static std::string infer_selections(const std::string& archive_path,
                                        const std::string& mod_path);

    static std::filesystem::path resolve_mod_archive(const std::string& installation_file,
                                                     const std::filesystem::path& mod_folder,
                                                     const std::filesystem::path& mods_dir);
};

}  // namespace mo2server

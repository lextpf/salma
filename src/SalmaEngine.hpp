#pragma once

#include <filesystem>
#include <string>

/**
 * Bridge from the Crow server to the Rust engine DLL.
 *
 * The engine used to be C++ classes compiled into this executable
 * (`mo2core::InstallationService`, `mo2core::FomodInferenceService`,
 * `mo2core::resolve_mod_archive`). It is now `mo2-salma.dll`, built from the
 * Rust crate at the repo root, and reached through the same flat `extern "C"`
 * ABI the MO2 Python plugin uses. This class is the only place in the server
 * that knows that.
 *
 * The DLL is loaded lazily on first use and never unloaded: it is process-wide
 * state with a registered log callback, so unloading it under a live request
 * would invalidate that callback.
 *
 * @ingroup Server
 */
namespace mo2server
{

class SalmaEngine
{
  public:
    /**
     * Load the engine DLL if it is not already loaded.
     *
     * Searches, in order: next to the server executable, an adjacent `salma/`
     * subdirectory (the layout `deploy.bat` produces), then the default OS
     * search path. Safe to call repeatedly; only the first call does work.
     *
     * @return true when the DLL is loaded and every required export resolved.
     */
    static bool ensure_loaded();

    /**
     * Path the engine DLL was loaded from, or an empty string when it has not
     * been loaded. Intended for diagnostics and the dashboard status endpoint.
     */
    static std::string loaded_path();

    /**
     * Engine ABI version string (`getApiVersion`), or "" when unavailable.
     */
    static std::string api_version();

    /**
     * Install a mod. Mirror of the former `InstallationService::install_mod`,
     * including its failure mode: the C++ threw on failure and the callers
     * catch `std::exception`, so a failed install throws here rather than
     * returning an error string.
     *
     * @param archive_path Archive to install from.
     * @param mod_path Destination mod directory.
     * @param json_path Selections JSON; empty means "derive the sidecar".
     * @return The installed mod path reported by the engine.
     * @throws std::runtime_error when the engine reports failure, carrying the
     *         engine's own message, or when the DLL cannot be loaded.
     */
    static std::string install_mod(const std::string& archive_path,
                                   const std::string& mod_path,
                                   const std::string& json_path);

    /**
     * Infer FOMOD selections. Mirror of
     * `FomodInferenceService::infer_selections`: returns schema-v2 JSON, or an
     * empty string on ANY failure. Never throws.
     */
    static std::string infer_selections(const std::string& archive_path,
                                        const std::string& mod_path);

    /**
     * Resolve a mod's source archive. Mirror of `mo2core::resolve_mod_archive`:
     * returns an empty path when nothing resolves. Never throws.
     */
    static std::filesystem::path resolve_mod_archive(const std::string& installation_file,
                                                     const std::filesystem::path& mod_folder,
                                                     const std::filesystem::path& mods_dir);
};

}  // namespace mo2server

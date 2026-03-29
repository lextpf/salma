#pragma once

#include <filesystem>
#include <mutex>
#include <string>

namespace mo2server
{

/**
 * @class ConfigService
 * @brief Singleton that reads/writes salma.json configuration.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ConfigService
 *
 * Persists the MO2 mods directory path (`mo2ModsPath`) to a
 * `salma.json` file next to the executable.  This is currently the
 * only persisted setting -- the FOMOD output directory is derived at
 * runtime as `{mo2ModsPath}/Salma FOMODs Output/fomods/`.
 *
 * ## :material-content-save-outline: Persistence Model
 *
 * - load() reads `salma.json` on startup; missing file is not an error.
 * - save() must be called explicitly -- configuration changes made via
 *   set_mo2_mods_path() are **not** auto-persisted.
 * - The JSON schema is a flat object: `{ "mo2ModsPath": "..." }`.
 *
 * ## :material-help: Thread Safety
 *
 * All public methods except config_path() are mutex-guarded.  Safe
 * to call from any thread, including Crow request handlers.
 * config_path() returns an effectively immutable path set once in
 * the constructor, so it is safe to call without locking.
 *
 * ## :material-microsoft-windows: Platform Note
 *
 * The config file path is resolved via `GetModuleFileNameW` -- this
 * class is Windows-only.
 */
class ConfigService
{
public:
    static ConfigService& instance();

    /**
     * Load configuration from salma.json. Missing file is not an error
     * (defaults are used). Parse errors are logged and silently ignored --
     * the method never throws.
     */
    void load();

    /**
     * Persist current configuration to salma.json. Write errors are
     * logged -- the method never throws.
     *
     * @return `true` if the config was written successfully, `false`
     *         on any I/O failure (disk full, permissions, etc.).
     */
    bool save();

    /**
     * @brief Get the configured MO2 mods directory path.
     * @return The mods path string, or empty string if not configured.
     */
    std::string mo2_mods_path() const;

    /**
     * @brief Set the MO2 mods directory path (not auto-persisted).
     *
     * Call save() afterwards to persist the change to salma.json.
     *
     * @param path Absolute path to the MO2 mods directory.
     */
    void set_mo2_mods_path(const std::string& path);

    /**
     * @brief Get the derived FOMOD output directory.
     *
     * Computed as `{mo2ModsPath}/Salma FOMODs Output/fomods/`.
     *
     * @return The FOMOD output path, or an empty path if mo2_mods_path
     *         is not configured.
     */
    std::filesystem::path fomod_output_dir() const;

    /**
     * @brief Get the path to the salma.json config file.
     * @return Absolute path next to the executable. Effectively
     *         immutable after construction (safe to call without locking).
     */
    std::filesystem::path config_path() const;

private:
    ConfigService();
    ~ConfigService() = default;
    ConfigService(const ConfigService&) = delete;
    ConfigService& operator=(const ConfigService&) = delete;

    mutable std::mutex mutex_;
    std::string mo2_mods_path_;
    std::filesystem::path config_path_;
};

}  // namespace mo2server

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
 * - save() persists the in-memory state via a write-then-rename so the
 *   on-disk file is never observed half-written.
 * - apply_mo2_mods_path() is the transactional setter: it stages the new
 *   value, calls save(), and reverts the in-memory state on failure so
 *   the running process can never disagree with what is on disk.
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
     * the method never throws. Loaded paths are NOT validated here; use
     * is_mo2_mods_path_valid() to surface configuration errors.
     */
    void load();

    /**
     * Persist current configuration to salma.json via a write-then-rename
     * so a partial write cannot leave a corrupted file on disk. Write
     * errors are logged -- the method never throws.
     *
     * @return `true` if the config was written and renamed atomically,
     *         `false` on any I/O failure (disk full, permissions, etc.).
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
     * Prefer apply_mo2_mods_path() over this + save() because the
     * transactional helper rolls back on save failure.
     *
     * @param path Absolute path to the MO2 mods directory.
     */
    void set_mo2_mods_path(const std::string& path);

    /**
     * @brief Atomically set + persist the MO2 mods directory path.
     *
     * Stages @p path in memory, attempts save(), and reverts to the
     * previous value if save() fails. Guarantees that the in-memory
     * state and the on-disk salma.json never diverge.
     *
     * @param path Absolute path to the MO2 mods directory.
     * @return `true` if both the memory and disk update succeeded;
     *         `false` if save() failed (memory state is unchanged).
     */
    bool apply_mo2_mods_path(const std::string& path);

    /**
     * @brief Whether the configured MO2 mods path points at an existing
     *        directory.
     *
     * Used to surface stale configuration: a config file may reference a
     * path that has since been moved or deleted. Returns false when no
     * path is configured.
     */
    bool is_mo2_mods_path_valid() const;

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

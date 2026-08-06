#pragma once

#include <filesystem>
#include <mutex>
#include <string>

namespace mo2server
{

/**
 * @class ConfigService
 * @brief Stores the server configuration in salma.json.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup ConfigService
 *
 * The only stored key is `mo2ModsPath`. The FOMOD output directory is derived as
 * `{mo2ModsPath}/Salma FOMODs Output/fomods/`. Writes use a sibling temporary file
 * and rename it over the target.
 *
 * ### :material-transit-connection-variant: Persistence flow
 *
 * ```mermaid
 * flowchart LR
 *     stage[stage value] --> temp[write sibling temp]
 *     temp --> rename[rename over salma.json]
 *     rename -->|success| keep[keep value]
 *     temp -->|failure| restore[restore prior value]
 *     rename -->|failure| restore
 * ```
 *
 * ### :material-lock-outline: Thread safety
 *
 * Readers are synchronized. `apply_mo2_mods_path` uses separate lock scopes to
 * stage, save, and restore the value.
 *
 * @warning Callers must serialize configuration writes. Concurrent writers can
 *          restore stale state after a failed save.
 * @see mo2core::executable_directory
 */
class ConfigService
{
public:
    /**
     * @fn ConfigService& ConfigService::instance()
     * @brief Defers disk access until explicit startup loading.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Construction does not load the file. Call `load` during startup.
     *
     * @return The instance, valid until process exit.
     */
    static ConfigService& instance();

    /**
     * @fn void ConfigService::load()
     * @brief Leaves live state unchanged when input cannot be parsed.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A missing, unreadable, or invalid file leaves the current value unchanged.
     * Loaded paths are not rejected. Filesystem probes outside the guarded parse
     * can still raise a standard exception.
     */
    void load();

    /**
     * @fn bool ConfigService::save()
     * @brief Replaces the target through a sibling temporary file.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A failed write or rename removes the temporary file and returns `false`.
     * Lock and allocation failures can propagate.
     *
     * @return `true` after the temporary file replaces the target.
     */
    bool save();

    /**
     * @fn std::string ConfigService::mo2_mods_path() const
     * @brief Read the configured mods path under the configuration mutex.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return A copy of the unvalidated path, or empty when unset.
     */
    std::string mo2_mods_path() const;

    /**
     * @fn void ConfigService::set_mo2_mods_path(const std::string& path)
     * @brief Stages an unvalidated value without disk access.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The value is not validated or persisted.
     *
     * @param path Path to store verbatim.
     */
    void set_mo2_mods_path(const std::string& path);

    /**
     * @fn bool ConfigService::apply_mo2_mods_path(const std::string& path)
     * @brief Restores the prior value when persistence fails.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A failed save restores the previous in-memory value.
     *
     * @param path Path to store verbatim.
     * @return `true` when memory and disk contain the new value.
     * @pre No other thread writes configuration during the call.
     */
    bool apply_mo2_mods_path(const std::string& path);

    /**
     * @fn bool ConfigService::is_mo2_mods_path_valid() const
     * @brief Fails closed for empty, inaccessible, or non-directory paths.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return `false` for an empty, missing, non-directory, or inaccessible path.
     */
    bool is_mo2_mods_path_valid() const;

    /**
     * @fn std::filesystem::path ConfigService::fomod_output_dir() const
     * @brief Keeps generated choices below the configured mods root.
     * @author Alex (<https://github.com/lextpf>)
     *
     * This call does not create or inspect the directory.
     *
     * @return The output path, or an empty path when no mods path is set.
     */
    std::filesystem::path fomod_output_dir() const;

    /**
     * @fn std::filesystem::path ConfigService::config_path() const
     * @brief Report the startup-resolved persistence location.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return The salma.json path, without creating or reading the file.
     */
    std::filesystem::path config_path() const;

private:
    /**
     * @fn ConfigService::ConfigService()
     * @brief Resolve the configuration path without loading its contents.
     * @author Alex (<https://github.com/lextpf>)
     */
    ConfigService();
    /**
     * @fn ConfigService::~ConfigService()
     * @brief Release the in-memory configuration without saving it.
     * @author Alex (<https://github.com/lextpf>)
     */
    ~ConfigService() = default;
    ConfigService(const ConfigService&) = delete;
    ConfigService& operator=(const ConfigService&) = delete;

    mutable std::mutex mutex_;
    std::string mo2_mods_path_;
    std::filesystem::path config_path_;
};

}  // namespace mo2server

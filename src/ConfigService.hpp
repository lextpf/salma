#pragma once

#include <filesystem>
#include <mutex>
#include <string>

namespace mo2server
{

/**
 * @class ConfigService
 * @brief stores the server configuration in salma.json.
 * @author Alex (https://github.com/lextpf)
 * @ingroup ConfigService
 *
 * the only stored key is `mo2ModsPath`. the FOMOD output directory is derived as
 * `{mo2ModsPath}/Salma FOMODs Output/fomods/`. writes use a sibling temporary file
 * and rename it over the target.
 *
 * ### :material-transit-connection-variant: persistence flow
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
 * ### :material-lock-outline: thread safety
 *
 * readers are synchronized. `apply_mo2_mods_path` uses separate lock scopes to
 * stage, save, and restore the value.
 *
 * @warning callers must serialize configuration writes. concurrent writers can
 *          restore stale state after a failed save.
 * @see mo2core::executable_directory
 */
class ConfigService
{
public:
    /**
     * @fn ConfigService& ConfigService::instance()
     * @brief defers disk access until explicit startup loading.
     * @author Alex (https://github.com/lextpf)
     *
     * construction does not load the file. call `load` during startup.
     *
     * @return the instance, valid until process exit.
     */
    static ConfigService& instance();

    /**
     * @fn void ConfigService::load()
     * @brief leaves live state unchanged when input cannot be parsed.
     * @author Alex (https://github.com/lextpf)
     *
     * a missing, unreadable, or invalid file leaves the current value unchanged.
     * loaded paths are not rejected. filesystem probes outside the guarded parse
     * can still raise a standard exception.
     */
    void load();

    /**
     * @fn bool ConfigService::save()
     * @brief replaces the target through a sibling temporary file.
     * @author Alex (https://github.com/lextpf)
     *
     * a failed write or rename removes the temporary file and returns `false`.
     * lock and allocation failures can propagate.
     *
     * @return `true` after the temporary file replaces the target.
     */
    bool save();

    std::string mo2_mods_path() const;

    /**
     * @fn void ConfigService::set_mo2_mods_path(const std::string& path)
     * @brief stages an unvalidated value without disk access.
     * @author Alex (https://github.com/lextpf)
     *
     * the value is not validated or persisted.
     *
     * @param path path to store verbatim.
     */
    void set_mo2_mods_path(const std::string& path);

    /**
     * @fn bool ConfigService::apply_mo2_mods_path(const std::string& path)
     * @brief restores the prior value when persistence fails.
     * @author Alex (https://github.com/lextpf)
     *
     * a failed save restores the previous in-memory value.
     *
     * @param path path to store verbatim.
     * @return `true` when memory and disk contain the new value.
     * @pre no other thread writes configuration during the call.
     */
    bool apply_mo2_mods_path(const std::string& path);

    /**
     * @fn bool ConfigService::is_mo2_mods_path_valid() const
     * @brief fails closed for empty, inaccessible, or non-directory paths.
     * @author Alex (https://github.com/lextpf)
     *
     * @return `false` for an empty, missing, non-directory, or inaccessible path.
     */
    bool is_mo2_mods_path_valid() const;

    /**
     * @fn std::filesystem::path ConfigService::fomod_output_dir() const
     * @brief keeps generated choices below the configured mods root.
     * @author Alex (https://github.com/lextpf)
     *
     * this call does not create or inspect the directory.
     *
     * @return the output path, or an empty path when no mods path is set.
     */
    std::filesystem::path fomod_output_dir() const;

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

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
 * Persists the MO2 mods directory path (`mo2ModsPath`) to `salma.json` next to
 * the executable. That is the only persisted setting; the FOMOD output
 * directory is derived at runtime as
 * `{mo2ModsPath}/Salma FOMODs Output/fomods/`.
 *
 * ## :material-content-save-outline: Persistence Model
 *
 * - The JSON schema is a flat object: `{ "mo2ModsPath": "..." }`.
 * - load() reads `salma.json` on startup. A missing file is not an error.
 * - save() writes then renames, so no reader ever observes a half-written file.
 * - apply_mo2_mods_path() is the transactional setter: stage, save, and revert
 *   the in-memory value if save() fails.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * sequenceDiagram
 *     participant H as Crow handler
 *     participant S as ConfigService
 *     participant M as mutex_
 *     participant D as salma.json
 *     H->>S: apply_mo2_mods_path(p)
 *     S->>M: lock 1 - stage p, unlock
 *     S->>M: lock 2 - enter save()
 *     S->>D: write salma.json.tmp, rename over salma.json
 *     S->>M: leave save(), unlock
 *     alt save() returned true
 *         S-->>H: true
 *     else save() returned false
 *         S->>M: lock 3 - restore the previous value, unlock
 *         S-->>H: false
 *     end
 * ```
 *
 * ## :material-help: Thread Safety
 *
 * Every public method except instance() and config_path() takes `mutex_`. Those
 * two are safe without it: instance() is serialized by the C++11 magic static,
 * and config_path() returns a value set once in the constructor.
 *
 * save() holds `mutex_` across its file I/O, so a concurrent mo2_mods_path() or
 * fomod_output_dir() blocks for the whole write and rename.
 *
 * **Single-writer precondition.** apply_mo2_mods_path() takes and releases
 * `mutex_` three separate times, once each to stage, save and roll back, as the
 * diagram above shows. A concurrent set_mo2_mods_path() or a second
 * apply_mo2_mods_path() can interleave in either gap; the unconditional
 * rollback then writes the previous value over whatever the other writer
 * staged, leaving the in-memory value matching neither the file on disk nor
 * either caller's intent. Concurrent readers are safe. Concurrent writers must
 * be serialized by the caller. Closing the gap needs a private
 * save-while-locked helper so stage, save and roll back run under one lock.
 *
 * ## :material-microsoft-windows: Platform Note
 *
 * The config file path comes from `mo2core::executable_directory()`, which
 * resolves the running executable's directory on Windows and falls back to the
 * process working directory elsewhere. The class is portable either way; on a
 * non-Windows build `salma.json` lands next to the working directory instead of
 * next to the binary.
 *
 * @see mo2core::executable_directory
 */
class ConfigService
{
public:
    /**
     * @brief Get the singleton ConfigService instance.
     *
     * The first call constructs the instance and resolves config_path().
     * Construction reads no file: call load() afterwards to pick up
     * `salma.json`.
     *
     * Thread-safe via the C++11 magic static, so concurrent first calls block
     * until construction finishes.
     *
     * @return Reference to the process-wide instance, valid until process exit.
     */
    static ConfigService& instance();

    /**
     * @brief Load configuration from salma.json.
     *
     * A missing file is not an error: the absence is logged and the defaults
     * stay in place. Parse errors are logged and ignored, so a corrupt file
     * leaves the defaults in place too.
     *
     * A loaded path is never rejected here. One that no longer points at an
     * existing directory is logged as a warning so the drift is visible; gate
     * behavior on is_mo2_mods_path_valid() at the call site instead.
     *
     * @throw Parse and open failures are contained and never propagate, but
     *        this is not an unconditional no-throw. The lock acquisition, the
     *        `exists` probe on the config path and the log formatting all run
     *        before the guarded block, so a lock failure or a filesystem error
     *        on the probe (an unreachable network path, a permission failure on
     *        the parent directory) propagates as `std::system_error` or
     *        `std::filesystem_error`. The guarded block catches only
     *        `std::exception`.
     */
    void load();

    /**
     * @brief Persist the current configuration to salma.json.
     *
     * Writes a sibling temp file and renames it over the target, so a partial
     * write cannot corrupt the config and no reader ever sees it half-written.
     * The temp file is removed when either the write or the rename fails. I/O
     * errors are logged and reported through the return value.
     *
     * @return `true` when the config was written and renamed, `false` on any
     *         I/O failure (disk full, permissions, and so on).
     * @throw I/O failures are contained and reported as `false`. As for load(),
     *        the lock acquisition happens before the guarded block and that
     *        block catches only `std::exception`, so this is not an
     *        unconditional no-throw guarantee.
     */
    bool save();

    /**
     * @brief Get the configured MO2 mods directory path.
     * @return The mods path string, or empty string if not configured. The
     *         value is a copy taken under the lock, so it is safe to keep.
     */
    std::string mo2_mods_path() const;

    /**
     * @brief Set the MO2 mods directory path in memory only.
     *
     * The path is stored verbatim: not validated, canonicalized or checked for
     * existence. Nothing reaches disk until save() runs. Prefer
     * apply_mo2_mods_path(), which rolls the memory value back when the save
     * fails.
     *
     * @param path Absolute path to the MO2 mods directory.
     */
    void set_mo2_mods_path(const std::string& path);

    /**
     * @brief Set and persist the MO2 mods directory path in one step.
     *
     * Stages @p path in memory, calls save(), and reverts to the previous value
     * when save() fails.
     *
     * @param path Absolute path to the MO2 mods directory.
     * @return `true` when memory and disk both updated. `false` when save()
     *         failed, in which case the in-memory value is back to what it was
     *         before the call.
     * @pre No other thread writes the configuration concurrently. Stage, save
     *      and roll back span three separate lock scopes, so "memory and disk
     *      agree" only holds for a single writer. See the Thread Safety section
     *      of the class.
     * @throw Same contract as save(), which this method calls.
     */
    bool apply_mo2_mods_path(const std::string& path);

    /**
     * @brief Whether the configured MO2 mods path points at an existing
     *        directory.
     *
     * Surfaces stale configuration: a config file can name a path that has
     * since been moved or deleted. The answer is a snapshot, and the directory
     * can disappear immediately afterwards.
     *
     * Returns `false` in all of these cases:
     * - No path is configured (empty string).
     * - The path is configured but does not exist.
     * - The path exists but is not a directory.
     * - `fs::is_directory` produced an `error_code` (e.g. permission
     *   denied during the stat call).
     *
     * @throw Does not throw on filesystem errors: the `error_code` overload is
     *        used and every failure is reported as `false`.
     */
    bool is_mo2_mods_path_valid() const;

    /**
     * @brief Get the derived FOMOD output directory.
     *
     * Computed as `{mo2ModsPath}/Salma FOMODs Output/fomods/`. The directory
     * is not created and its existence is not checked.
     *
     * @return The FOMOD output path, or an empty path if mo2_mods_path
     *         is not configured.
     */
    std::filesystem::path fomod_output_dir() const;

    /**
     * @brief Get the path to the salma.json config file.
     * @return Absolute path next to the executable. Set once in the
     *         constructor and never written again, so it is safe to call
     *         without locking.
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

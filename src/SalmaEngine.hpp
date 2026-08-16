#pragma once

#include <filesystem>
#include <string>

namespace mo2server
{

/**
 * @class SalmaEngine
 * @brief Bridge from the Crow server to the Rust engine DLL.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Server
 *
 * Install, inference and archive resolution all live in `mo2-salma.dll`, behind
 * the flat `extern "C"` ABI the MO2 Python plugin also binds. This class is the
 * only place in the server that knows that, and it adds the two things the flat
 * ABI cannot express:
 *
 * 1. **A failed install throws.** The ABI reports failure as an error string
 *    plus a false `installSucceeded()`. install_mod() turns that into a
 *    `std::runtime_error` carrying the engine's own message, because the Crow
 *    controllers report install failures by catching `std::exception`.
 * 2. **Installs are serialized process-wide.** `installSucceeded()` is one
 *    process-global flag inside the engine, so reading it is only meaningful
 *    while no second install can run. The server starts installs on overlapping
 *    background jobs, so install_mod() holds a process-wide mutex across both
 *    the call and the flag read.
 *
 * Every method is static. There is no instance to construct: the module handle,
 * the bound export pointers and the mutexes are file-scope process state in
 * SalmaEngine.cpp.
 *
 * ## :material-help: Thread Safety
 *
 * All methods are safe to call from any thread, including Crow request handlers
 * and BackgroundJob workers.
 *
 * - Loading takes its own mutex, so concurrent first calls load the DLL once.
 * - install_mod() additionally takes the install mutex, so a second install
 *   blocks for the full duration of the first.
 * - infer_selections() and resolve_mod_archive() are not serialized against
 *   each other or against an install. They may run concurrently.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * sequenceDiagram
 *     participant A as BackgroundJob A
 *     participant B as BackgroundJob B
 *     participant M as install mutex
 *     participant E as mo2-salma.dll
 *     A->>M: lock
 *     A->>E: installWithConfig(...)
 *     B->>M: lock (blocks)
 *     E-->>A: result string (engine-allocated)
 *     A->>E: installSucceeded()
 *     E-->>A: bool
 *     A->>E: freeResult(result)
 *     A->>M: unlock
 *     B->>E: installWithConfig(...)
 * ```
 *
 * ## :material-cube: Loading
 *
 * The DLL is loaded lazily on first use and never unloaded. It holds
 * process-wide state including a registered log callback, and unloading it
 * under a live request would invalidate that callback.
 *
 * The search order is a deployment contract: `deploy.bat`, `run.bat` and the
 * MO2 Python plugin's `find_dll` must all agree with it.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart TD
 *     A["ensure_loaded()"] --> B{"already loaded?"}
 *     B -- yes --> Z["return true"]
 *     B -- no --> C["exe_dir/mo2-salma.dll"]
 *     C -- "exists and loads" --> E["bind 6 exports"]
 *     C -- miss --> D["exe_dir/salma/mo2-salma.dll"]
 *     D -- "exists and loads" --> E
 *     D -- miss --> F["LoadLibraryW default search order"]
 *     F -- loads --> E
 *     F -- miss --> X["log error, return false"]
 *     E -- "an export is missing" --> X
 *     E -- "all resolved" --> Y["mark loaded, record path, log API version"]
 *     X --> R["not cached: the next call repeats the whole search"]
 * ```
 *
 * The six bound exports are `installWithConfig`, `inferFomodSelections`,
 * `resolveModArchive`, `freeResult`, `installSucceeded` and `getApiVersion`.
 * `src/capi.rs` is the authority for their signatures. The declarations here and
 * the ctypes declarations in the MO2 plugin must both track it.
 *
 * ## :material-alert-circle-outline: Failure Behavior
 *
 * Every method calls ensure_loaded() first, so a missing or broken DLL surfaces
 * at each of them. install_mod() throws; the other methods return an empty
 * result.
 *
 * On a non-Windows build ensure_loaded() always logs an error and returns
 * false, so the class degrades to "engine unavailable" instead of failing to
 * compile.
 *
 * Every string the engine returns is engine-allocated. This class copies it
 * into a `std::string` and releases the original through `freeResult` before
 * returning, so no caller ever owns engine memory.
 */
class SalmaEngine
{
  public:
    /**
     * @brief Load the engine DLL if it is not already loaded.
     *
     * Searches, in order: next to the server executable, an adjacent `salma/`
     * subdirectory (the layout `deploy.bat` produces), then the default OS
     * search path. All six exports must resolve; one missing export fails the
     * whole load.
     *
     * Safe to call repeatedly and concurrently. The work happens once per
     * process, but only after a load succeeds. A failed load is not cached:
     * every later call repeats the full search (two `exists` probes, up to
     * three `LoadLibraryW` attempts) and logs the same error again. With no DLL
     * present that repeats on every install, infer and resolve request.
     *
     * **Blocking:** holds the load mutex for the duration of the check, so a
     * concurrent first call waits for the load to finish.
     *
     * @return `true` when the DLL is loaded and every required export
     *         resolved. Always `false` on a non-Windows build.
     */
    static bool ensure_loaded();

    /**
     * @brief Report where the engine DLL was loaded from.
     *
     * Does not trigger a load. The value is recorded only when a load succeeds.
     *
     * **One of three values:**
     *
     * - The absolute path of the DLL, when it was found next to the executable
     *   or in the adjacent `salma/` directory.
     * - The fixed string `mo2-salma.dll (default search path)`, when the OS
     *   default search order supplied it. That is a diagnostic label, not a
     *   filesystem path: do not stat, hash or open it.
     * - The empty string, when no load has succeeded yet.
     *
     * Consumed only by tests/salma_engine_test.cpp. Wire it into
     * `/api/mo2/status` if the dashboard should report the engine build.
     *
     * @return The recorded load location, as described above.
     */
    static std::string loaded_path();

    /**
     * @brief Report the engine ABI version string.
     *
     * Triggers a load if one has not happened yet. The value comes from the
     * `getApiVersion` export, the one export whose result is not heap-allocated
     * and must not be freed.
     *
     * Consumed only by tests/salma_engine_test.cpp.
     *
     * @return The version string (for example "1.2.0"), or an empty string
     *         when the DLL cannot be loaded or the export returns null.
     */
    static std::string api_version();

    /**
     * @brief Install a mod through the engine.
     *
     * Failure is an exception, not a return value: the engine reports its error
     * text in the same slot as the success value, and that text becomes the
     * `what()` of a `std::runtime_error`.
     *
     * **Serialized process-wide.** The call and the `installSucceeded()` flag
     * read happen under one mutex, because that flag is a single process-global
     * value in the engine. A second install blocks until the first returns. The
     * Thread Safety section of the class shows the sequence.
     *
     * @param archive_path Archive to install from.
     * @param mod_path Destination mod directory.
     * @param json_path Selections JSON. An explicit path is passed to the
     *        engine as given and is not checked for existence here. An empty
     *        string means "derive the sidecar": the engine looks for
     *        `<archive stem>.json` next to the archive, uses it only if that
     *        file exists and resolves inside the archive's parent directory,
     *        and otherwise installs with no selections at all.
     * @return The installed mod path reported by the engine.
     * @throw std::runtime_error when the engine reports failure, carrying the
     *        engine's own message, or when the DLL cannot be loaded.
     */
    static std::string install_mod(const std::string& archive_path,
                                   const std::string& mod_path,
                                   const std::string& json_path);

    /**
     * @brief Infer the FOMOD selections that produced an installed mod.
     *
     * Compares the archive's FOMOD options against the already-installed tree
     * and returns schema-v2 JSON.
     *
     * Not serialized: may run concurrently with another inference and with an
     * install.
     *
     * @param archive_path Source archive to read the FOMOD from.
     * @param mod_path Installed mod directory to compare against.
     * @return Schema-v2 JSON, or an empty string on any failure, including a
     *         DLL that cannot be loaded. Engine failures are never exceptions,
     *         so callers need no catch for them. The function is not `noexcept`
     *         and it allocates, so allocation and lock failures can still
     *         propagate.
     */
    static std::string infer_selections(const std::string& archive_path,
                                        const std::string& mod_path);

    /**
     * @brief Resolve a mod's source archive.
     *
     * Walks a fixed candidate chain (the value itself if absolute, the
     * configured downloads root, the mod folder, then directories above the
     * mods root) and returns the first candidate that exists. The chain lives
     * in the engine, in `src/archive_resolver.rs`.
     *
     * Not serialized: may run concurrently with an install.
     *
     * @param installation_file The `installationFile` value recorded for the
     *        mod. May be a bare file name or a path.
     * @param mod_folder The installed mod directory.
     * @param mods_dir The MO2 mods root.
     * @return The resolved archive path, or an empty path when nothing
     *         resolves, including when the DLL cannot be loaded. As for
     *         infer_selections(), engine failures are never exceptions, but the
     *         function is not `noexcept` and allocation failures can propagate.
     */
    static std::filesystem::path resolve_mod_archive(const std::string& installation_file,
                                                     const std::filesystem::path& mod_folder,
                                                     const std::filesystem::path& mods_dir);
};

}  // namespace mo2server

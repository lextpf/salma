#pragma once

#include "Export.h"

/**
 * @namespace CApi
 * @brief C-linkage API exported by the salma DLL.
 * @author Alex (https://github.com/lextpf)
 * @ingroup CApi
 *
 * Flat C interface consumed by the MO2 Python plugin (via `ctypes`) and
 * any other language that can call `extern "C"` functions. Every function
 * in this header is exported with `MO2_API` (`__declspec(dllexport)` on
 * Windows) so they appear in the DLL's export table.
 *
 * ## :material-layers-outline: Architecture
 *
 * The C API is a thin wrapper - each function constructs the appropriate
 * C++ service, forwards the call, and returns a heap-allocated copy of
 * the result via `_strdup()`. The caller owns the returned pointer and
 * must release it with `freeResult()`.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart LR
 *     classDef caller fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
 *     classDef api fill:#134e3a,stroke:#10b981,color:#e2e8f0
 *     classDef svc fill:#2e1f5e,stroke:#8b5cf6,color:#e2e8f0
 *     classDef out fill:transparent,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
 *
 *     PY[Python plugin]:::caller
 *     API[CApi]:::api
 *
 *     subgraph Services
 *         direction TB
 *         IS[InstallationService]:::svc
 *         FIS[FomodInferenceService]:::svc
 *     end
 *
 *     RES[const char*]:::out
 *
 *     PY -->|ctypes| API
 *     API -->|install| IS
 *     API -->|infer| FIS
 *     IS --> RES
 *     FIS --> RES
 * ```
 *
 * ## :material-code-tags: Usage Examples
 *
 * ### Python - load the DLL and install a mod
 * ```python
 * import ctypes
 * dll = ctypes.CDLL("mo2-salma.dll")
 * dll.install.restype = ctypes.c_char_p
 * result = dll.install(b"C:/downloads/mod.7z", b"C:/mods/MyMod")
 * ```
 *
 * ### Python - infer FOMOD selections
 * ```python
 * dll.inferFomodSelections.restype = ctypes.c_char_p
 * json_bytes = dll.inferFomodSelections(
 *     b"C:/downloads/mod.7z",
 *     b"C:/mods/MyMod",
 * )
 * selections = json.loads(json_bytes)
 * ```
 *
 * ### Python - redirect DLL log output
 * ```python
 * @ctypes.CFUNCTYPE(None, ctypes.c_char_p)
 * def on_log(msg):
 *     print(msg.decode())
 *
 * dll.setLogCallback(on_log)
 * ```
 *
 * ## :material-memory: Lifetime
 *
 * Every `const char*` returned by `install`, `installWithConfig`, or
 * `inferFomodSelections` is a heap-allocated string (`_strdup`).
 * The caller **must** call `freeResult()` to release the memory when
 * done. Failing to do so will leak memory.
 *
 * ## :material-help: Thread Safety
 *
 * Each entry point is reentrant and may be called from multiple
 * threads. Returned `const char*` strings are independent heap
 * allocations, so there is no shared output buffer.
 *
 * **Caveat -- shared "last install" flag.** `install()` /
 * `installWithConfig()` write a process-global success flag that
 * `installSucceeded()` reads under a mutex. If two threads run
 * concurrent installs, `installSucceeded()` reflects whichever one
 * finished last, not necessarily the caller's own install. The MO2
 * Python plugin avoids the race by reading `installSucceeded()` from
 * the same thread that called `install()`. Mixed-thread polling is
 * not supported.
 *
 * @see InstallationService, FomodInferenceService
 */
namespace CApi
{

/**
 * @brief Stable ABI version string returned by getApiVersion().
 *
 * Bumped on any breaking change to the exported function set or
 * argument/return semantics. Callers (notably the MO2 Python plugin)
 * compare this string against their expected major version to detect
 * deploy mismatches between a stale plugin and a newer DLL or vice
 * versa, before invoking any other entry point.
 *
 * Format is `MAJOR.MINOR.PATCH`. A different MAJOR is incompatible.
 *
 * 1.0.0 - initial public ABI.
 * 1.1.0 - added @ref resolveModArchive.
 * 1.2.0 - inferFomodSelections JSON wire format becomes schema_version=2:
 *         per-plugin objects (name + selected + confidence + reasons) and a
 *         top-level diagnostics block. Old saved selection files in the
 *         schema-v1 string-array form are still accepted by the install
 *         consumer for backward compatibility.
 */
#define MO2_SALMA_API_VERSION "1.2.0"
#define MO2_SALMA_API_MAJOR "1"

// Compile-time guard: bumping MO2_SALMA_API_VERSION without keeping
// MO2_SALMA_API_MAJOR's first segment in sync would silently break the
// callers' major-only ABI check.
static_assert(MO2_SALMA_API_VERSION[0] == MO2_SALMA_API_MAJOR[0] && MO2_SALMA_API_MAJOR[1] == '\0',
              "MO2_SALMA_API_MAJOR must equal the leading digit of "
              "MO2_SALMA_API_VERSION");

/**
 * @brief Signature for the log callback function.
 * @ingroup CApi
 *
 * The callback receives a null-terminated UTF-8 log message.
 * It is invoked synchronously on the calling thread whenever
 * the Logger produces output.
 */
typedef void (*Mo2LogCallback)(const char*);

extern "C"
{
    /**
     * @brief Return the DLL's stable ABI version string.
     * @ingroup CApi
     *
     * The returned pointer references a static constant inside the DLL.
     * It must NOT be passed to freeResult().
     *
     * @return Null-terminated UTF-8 version string in `MAJOR.MINOR.PATCH` form.
     */
    MO2_API const char* getApiVersion();

    /**
     * @brief Register a callback to receive log messages from the DLL.
     * @ingroup CApi
     *
     * Forwards the callback to Logger::set_callback(). Pass `nullptr`
     * to clear the callback and re-enable file logging (`logs/salma.log`).
     *
     * @param callback Function pointer, or `nullptr` to clear.
     */
    MO2_API void setLogCallback(Mo2LogCallback callback);

    /**
     * @brief Process a FOMOD-capable archive using automatic FOMOD detection.
     * @ingroup CApi
     *
     * Extracts the archive into a temporary directory, detects whether
     * a FOMOD installer is present, and runs the appropriate FOMOD replay
     * or fallback copy
     * path.
     * For FOMOD archives without a JSON config, only required files and conditional
     * patterns are installed - optional steps are skipped because no selections exist. For full
     * FOMOD installs with pre-scanned selections use installWithConfig() instead.
     *
     * @param archivePath Null-terminated UTF-8 path to the archive. Must not be `nullptr`.
     * @param modPath Null-terminated UTF-8 path to the output mod directory. Must not be `nullptr`.
     * @return Heap-allocated mod path string on success. On fatal error or null
     *         input the string contains the error message. The caller **must**
     *         call freeResult() to release the returned pointer.
     * @note All C++ exceptions are caught internally and returned as error
     *       strings -- callers will never see a C++ exception propagate.
     */
    MO2_API const char* install(const char* archivePath, const char* modPath);

    /**
     * @brief Replay a FOMOD archive using pre-scanned FOMOD selections.
     * @ingroup CApi
     *
     * Same as install(), but reads FOMOD step/option choices from a JSON
     * file. This is the primary entry point used by the MO2 plugin
     * after `inferFomodSelections` has produced a configuration file.
     * Without a JSON file, only required files and conditional patterns
     * are installed; with one, the selected optional plugins are
     * installed too.
     *
     * @param archivePath Null-terminated UTF-8 path to the archive. Must not be `nullptr`.
     * @param modPath Null-terminated UTF-8 path to the output mod directory. Must not be `nullptr`.
     * @param jsonPath Null-terminated UTF-8 path to the FOMOD selections
     *        JSON file. Pass `nullptr` or empty string to skip optional
     *        steps (equivalent to install()).
     * @return Heap-allocated mod path string on success. On fatal error or null
     *         input the string contains the error message. The caller **must**
     *         call freeResult() to release the returned pointer.
     * @note All C++ exceptions are caught internally and returned as error
     *       strings -- callers will never see a C++ exception propagate.
     */
    MO2_API const char* installWithConfig(const char* archivePath,
                                          const char* modPath,
                                          const char* jsonPath);

    /**
     * @brief Infer FOMOD selections by comparing an archive against an installed mod.
     * @ingroup CApi
     *
     * Reads the archive's FOMOD `ModuleConfig.xml`, walks every install
     * step, and matches each option's file list against the files already
     * present in @p modPath. Returns a JSON object describing which
     * options were selected in each step.
     *
     * Return cases:
     * - Both inputs non-null, success -> heap-allocated JSON selections.
     * - Both inputs non-null, archive has no FOMOD installer -> empty string.
     * - Both inputs non-null, inference pipeline throws internally -> empty
     *   string (the exception is caught and logged).
     * - Either @p archivePath or @p modPath is `nullptr` -> heap-allocated
     *   literal `"archivePath and modPath must not be null"` (not empty).
     *
     * Callers that distinguish "no FOMOD" from "usage error" must check
     * for the literal error string in addition to empty.
     *
     * @param archivePath Null-terminated UTF-8 path to the archive. Must not be `nullptr`.
     * @param modPath Null-terminated UTF-8 path to the installed mod
     *        directory to compare against. Must not be `nullptr`.
     * @return Heap-allocated JSON selections string. Empty string on
     *         pipeline failure; error-message string on null input.
     *         The caller **must** call freeResult() to release the returned pointer.
     * @note All C++ exceptions are caught internally and returned as an
     *       empty string -- callers will never see a C++ exception propagate.
     */
    MO2_API const char* inferFomodSelections(const char* archivePath, const char* modPath);

    /**
     * @brief Check whether the last install() or installWithConfig() call succeeded.
     *
     * @return true if the last result was a valid mod path (success), false if it
     *         was an error message.
     */
    MO2_API bool installSucceeded();

    /**
     * @brief Free a result string returned by install(), installWithConfig(),
     *        inferFomodSelections(), or resolveModArchive().
     *
     * Each call to those functions returns a heap-allocated string. The caller
     * **must** call freeResult() to release the memory when done.
     *
     * @param result Pointer previously returned by one of the listed API
     *        functions, or `nullptr` (no-op via `free(nullptr)`).
     * @warning Passing a pointer that was not returned by one of the listed
     *          API functions (e.g. a foreign allocation, a stack pointer, or
     *          a pointer that has already been freed) is **undefined
     *          behavior**. The implementation calls `free()` directly with
     *          no validation.
     * @warning `getApiVersion()` returns a pointer to a static constant
     *          inside the DLL; do **not** pass that pointer here.
     */
    MO2_API void freeResult(const char* result);

    /**
     * @brief Resolve a mod's source archive path using the same fallback
     *        chain as the dashboard's FOMOD scan.
     * @ingroup CApi
     * @since 1.1.0
     *
     * Centralizes the resolution rules so the local dashboard and the MO2
     * Python plugin agree on which archive a mod folder maps to. The
     * fallback chain is documented on
     * @ref mo2core::resolve_mod_archive (`SALMA_DOWNLOADS_PATH`,
     * `<mod_folder>`, `<mods_dir>/..`, then two `downloads/` sibling
     * lookups). Previously the Python plugin had a much shallower chain,
     * so a mod whose archive lived in `<mods_dir>/../downloads` would
     * succeed in the server scan but fail in the plugin.
     *
     * @param installationFile Null-terminated UTF-8 value from
     *        `[General] installationFile` in the mod's `meta.ini`.
     *        Must not be `nullptr`.
     * @param modFolder Null-terminated UTF-8 absolute path to the mod
     *        folder. Must not be `nullptr`.
     * @param modsDir Null-terminated UTF-8 absolute path to the parent
     *        mods directory (e.g. `<MO2 instance>/mods`). May be empty.
     * @return Heap-allocated absolute path on hit, or empty string on
     *         miss. The caller **must** call freeResult() to release the
     *         returned pointer.
     * @note All C++ exceptions are caught internally and surfaced as an
     *       empty result; callers will never see a C++ exception propagate.
     */
    MO2_API const char* resolveModArchive(const char* installationFile,
                                          const char* modFolder,
                                          const char* modsDir);

}  // extern "C"

}  // namespace CApi

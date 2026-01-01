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
 * C++ service, forwards the call, and copies the result into a static
 * `std::string` whose `.c_str()` is returned to the caller.
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
 * Returned `const char*` pointers are backed by `static std::string`
 * buffers. `install` and `installWithConfig` share one buffer;
 * `inferFomodSelections` uses a separate one. A pointer is valid
 * until the next call to any function sharing the same buffer.
 * Callers that need to keep the data should copy the string
 * immediately.
 *
 * ## :material-help: Thread Safety
 *
 * The API is **not** thread-safe. `install` and `installWithConfig`
 * share one static result buffer; `inferFomodSelections` uses a second.
 * Concurrent calls from different threads will race on those buffers.
 * The Python plugin serializes all calls through a single thread, so
 * this is not an issue in practice.
 *
 * @see InstallationService, FomodInferenceService
 */
namespace CApi
{

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
     * @brief Install a mod archive using automatic FOMOD detection.
     * @ingroup CApi
     *
     * Extracts the archive into a temporary directory, detects whether
     * a FOMOD installer is present, and runs the appropriate installation
     * path.
     *
     * For FOMOD archives without a JSON config, only required files and
     * conditional patterns are installed - optional steps are skipped
     * because no selections exist. For full FOMOD installs with
     * pre-scanned selections use installWithConfig() instead.
     *
     * @param archivePath Null-terminated UTF-8 path to the archive. Must not be `nullptr`.
     * @param modPath Null-terminated UTF-8 path to the output mod directory. Must not be `nullptr`.
     * @return The mod path string on success. Valid until the next
     *         `install` or `installWithConfig` call. On fatal error
     *         or null input the string contains the error message.
     */
    MO2_API const char* install(const char* archivePath, const char* modPath);

    /**
     * @brief Install a mod archive using pre-scanned FOMOD selections.
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
     * @return The mod path string on success. Valid until the next
     *         `install` or `installWithConfig` call. On fatal error
     *         or null input the string contains the error message.
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
     * Returns an empty string if the archive has no FOMOD installer or
     * if inference fails entirely.
     *
     * @param archivePath Null-terminated UTF-8 path to the archive.
     * @param modPath Null-terminated UTF-8 path to the installed mod
     *        directory to compare against.
     * @return JSON selections string. Valid until the next
     *         `inferFomodSelections` call. Empty string on failure.
     */
    MO2_API const char* inferFomodSelections(const char* archivePath, const char* modPath);

}  // extern "C"

}  // namespace CApi

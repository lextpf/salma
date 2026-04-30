#pragma once

#include <atomic>
#include <fstream>
#include <functional>
#include <mutex>
#include <string>
#include "Export.h"

namespace mo2core
{

/**
 * @brief Function pointer type for external log callbacks.
 *
 * The callback receives a null-terminated UTF-8 string containing the
 * raw log message (no timestamp or level prefix -- those are only added
 * to file output). The callback may be invoked from **any** thread that
 * calls a Logger method, and multiple threads may invoke it concurrently
 * without serialization. Implementations **must** be thread-safe.
 *
 * @see Logger::set_callback
 */
using LogCallback = void (*)(const char*);

/**
 * @class Logger
 * @brief Logging singleton with external callback support.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Logger
 *
 * Meyer's singleton that provides three log levels (info, warning,
 * error) with dual output: an optional external callback for host
 * applications (e.g. the MO2 Python plugin) and a local log file
 * (`logs/salma.log`) as fallback when no callback is registered.
 *
 * ## :material-message-text-outline: Output Routing
 *
 * | Callback? |    Console    |   File    | Callback |
 * |-----------|---------------|-----------|----------|
 * |        No | stdout/stderr | salma.log | -        |
 * |       Yes | stdout/stderr |     -     | invoked  |
 *
 * When a callback is registered via setLogCallback(), all messages
 * are forwarded to it and file logging is skipped (the host is
 * assumed to handle persistence). Console output (stdout for info
 * and warning, stderr for error) is always active regardless.
 *
 * ## :material-format-text: Log Format
 *
 * File entries are formatted as:
 * ```
 * 2026-02-28 19:45:02.123 INFO [install] message text
 * ```
 *
 * ## :material-code-tags: Usage Example
 *
 * ```cpp
 * auto& log = Logger::instance();
 * log.log("[archive] Extracting archive...");
 * log.log_warning("[fomod] Skipping missing ACL");
 * log.log_error("[install] Fatal: corrupt header");
 * ```
 *
 * ## :material-help: Thread Safety
 *
 * The singleton is initialized via a C++11 magic static.
 * File writes are mutex-guarded. `callback_` is an atomic pointer
 * (acquire/release), so log methods snapshot it lock-free.
 *
 * **Caveat:** console output (`std::cout` / `std::cerr`) is not
 * mutex-guarded -- concurrent log calls may produce interleaved
 * lines on the terminal.  File output is serialized by `mutex_`.
 * Callback dispatch is **not** serialized -- the atomic snapshot
 * ensures each call sees a consistent pointer, but multiple
 * threads may invoke the callback concurrently.  The callback
 * implementation **must** be thread-safe.
 *
 * **Callback failure:** Every callback invocation is wrapped in a
 * `catch (...)` block. If the callback throws, the exception is
 * caught internally and a diagnostic message is written to stderr.
 * Exceptions do **not** propagate to the calling log site.
 *
 * **Reentrancy:** The callback may be invoked while a previous
 * callback invocation is still running (from another thread).
 * The callback pointer is snapshotted under `mutex_`, but the
 * callback itself is invoked **outside** the lock, so calling
 * Logger methods from within a callback will not deadlock.
 * However, this is discouraged because it can cause infinite
 * recursion if the callback triggers another log message that
 * in turn invokes the callback again.
 *
 * ## :material-autorenew: Log Rotation
 *
 * The log file rotates when it reaches 10 MiB. Up to 3 rotated
 * files are kept (`salma.log.1` through `salma.log.3`). On
 * rotation, existing rotated files are shifted (`.2` becomes `.3`,
 * `.1` becomes `.2`), the oldest (`.3`) is deleted if present,
 * and the active `salma.log` is moved to `.1`.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * stateDiagram-v2
 *     [*] --> active: open salma.log
 *     active --> rotating: bytes_written_ >= 10 MiB
 *     rotating --> active: rename salma.log -> salma.log.1, shift .1->.2 .2->.3, delete old .3
 * ```
 *
 * @see CApi::setLogCallback
 */
class MO2_API Logger
{
public:
    /**
     * @brief Get the singleton Logger instance.
     *
     * First call initializes the instance (creates `logs/` directory).
     * Thread-safe via magic static.
     */
    static Logger& instance();

    /**
     * @brief Register an external log callback.
     *
     * When set, all log messages are forwarded to the callback and
     * file logging is disabled. Pass `nullptr` to revert to file
     * logging.
     *
     * The callback **must** be thread-safe -- it may be invoked
     * concurrently from multiple threads without serialization.
     *
     * @warning The callback pointer must remain valid until cleared
     * (by passing `nullptr`) or until the Logger singleton is destroyed.
     * Passing a dangling function pointer results in undefined behavior.
     *
     * @param callback Function pointer, or `nullptr` to clear.
     */
    void set_callback(LogCallback callback);

    /**
     * @brief Log an informational message.
     *
     * Output order: file (under mutex) if no callback registered,
     * stdout (outside lock), callback (if registered, outside lock).
     * Console output is intentionally outside the lock so the file
     * write is not held back by slow terminal I/O.
     *
     * @param message The log message (typically prefixed with a subsystem tag like `[install]`).
     */
    void log(const std::string& message);

    /**
     * @brief Log an error message.
     *
     * Output order: file (under mutex) if no callback registered,
     * stderr (outside lock), callback (if registered, outside lock).
     * Console output is intentionally outside the lock so the file
     * write is not held back by slow terminal I/O.
     *
     * @param message The error message.
     */
    void log_error(const std::string& message);

    /**
     * @brief Log a warning message.
     *
     * Output order: file (under mutex) if no callback registered,
     * stdout (outside lock), callback (if registered, outside lock).
     * Console output is intentionally outside the lock so the file
     * write is not held back by slow terminal I/O.
     *
     * @param message The warning message.
     */
    void log_warning(const std::string& message);

    /**
     * @brief Truncate the log file safely.
     *
     * Acquires the file-write mutex, closes the persistent file handle,
     * truncates the file, and reopens it in append mode. This avoids the
     * corruption that would occur if an external caller truncated the file
     * while the Logger still held an open handle.
     *
     * @return `true` if the file was successfully truncated and reopened.
     */
    bool clear_log();

    /**
     * @brief Return the path to the active log file.
     */
    std::string log_path() const;

    /**
     * @brief Return the path to the active log directory.
     *
     * Always anchored next to the module that owns Logger (the mo2-salma
     * library). Use this so consumers do not re-derive the path through
     * `current_path()` and end up disagreeing with what Logger writes.
     */
    std::string log_directory() const;

private:
    Logger();
    ~Logger();
    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    // Writes to the log file. Caller must hold mutex_.
    void write_log_unlocked(const std::string& level, const std::string& message);

    // Rotates the log file if bytes_written_ exceeds kMaxLogSize.
    // Caller must hold mutex_.
    void rotate_if_needed();

    static constexpr size_t kMaxLogSize = 10 * 1024 * 1024; /**< Rotate at 10 MiB */
    static constexpr int kMaxRotatedFiles = 3;              /**< Keep salma.log.1 through .3 */

    std::atomic<LogCallback> callback_ =
        nullptr;                /**< External callback (null = use file logging) */
    std::string log_directory_; /**< Absolute path to the logs directory */
    std::ofstream log_file_;    /**< Persistent log file handle (append mode) */
    std::mutex mutex_;          /**< Guards file writes */
    size_t bytes_written_ = 0;  /**< Approximate bytes written since last rotation */
};

}  // namespace mo2core

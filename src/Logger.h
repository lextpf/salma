#pragma once

#include <atomic>
#include <fstream>
#include <functional>
#include <mutex>
#include <string>
#include "Export.h"

namespace mo2core
{

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
 * **Callback failure:** If the callback throws or crashes, the
 * exception propagates to the calling log site. There is no
 * internal fallback to file logging when the callback fails.
 * Callers registering a callback should ensure it does not throw.
 *
 * **Reentrancy:** The callback may be invoked while a previous
 * callback invocation is still running (from another thread).
 * It must not call Logger methods, as file-write paths acquire
 * `mutex_` and would deadlock if the callback itself is on the
 * file-logging code path.
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
     * @param callback Function pointer, or `nullptr` to clear.
     */
    void set_callback(LogCallback callback);

    /**
     * @brief Log an informational message.
     *
     * Writes to stdout, then to callback or file.
     *
     * @param message The log message (typically prefixed with a subsystem tag like `[install]`).
     */
    void log(const std::string& message);

    /**
     * @brief Log an error message.
     *
     * Writes to stderr, then to callback or file.
     *
     * @param message The error message.
     */
    void log_error(const std::string& message);

    /**
     * @brief Log a warning message.
     *
     * Writes to stdout, then to callback or file.
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

private:
    Logger();
    ~Logger();
    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    void write_log(const std::string& level, const std::string& message);

    std::atomic<LogCallback> callback_ = nullptr;  ///< External callback (null = use file logging)
    std::string log_directory_;                    ///< Absolute path to the logs directory
    std::ofstream log_file_;                       ///< Persistent log file handle (append mode)
    std::mutex mutex_;                             ///< Guards file writes
};

}  // namespace mo2core

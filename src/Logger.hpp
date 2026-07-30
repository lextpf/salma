#pragma once

#include <atomic>
#include <fstream>
#include <functional>
#include <mutex>
#include <string>
#include "Export.hpp"

namespace mo2core
{

/**
 * @brief Receives one raw UTF-8 log message.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The string is valid only during the call. Callbacks can run concurrently on
 * caller threads and must remain valid until all in-flight calls finish.
 *
 * @see Logger::set_callback
 */
using LogCallback = void (*)(const char*);

/**
 * @class Logger
 * @brief Writes process logs to a callback or rotating file.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Logger
 *
 * ### :material-transit-connection-variant: Output routing
 *
 * | Callback | File output      | Callback output |
 * |----------|------------------|-----------------|
 * | Null     | `logs/salma.log` | None            |
 * | Set      | None             | Caller thread   |
 *
 * ```mermaid
 * flowchart LR
 *     message --> callback{callback set?}
 *     callback -- no --> file[one file write and flush]
 *     callback -- yes --> host[host callback]
 *     message --> console[console]
 *     file --> rotate{writer count at least 10 MiB?}
 *     rotate -- yes --> files[rotate three files]
 * ```
 *
 * ### :material-lock-outline: Thread safety
 *
 * Console output is always active. File writes are serialized. Callback calls
 * can overlap and run outside the file mutex. Callback exceptions are contained,
 * and same-thread recursive callbacks are dropped.
 *
 * The C++ and Rust loggers can append to the same file. Each complete record,
 * including its newline, must use one write followed by a flush. Splitting a
 * record permits the other writer to interleave bytes.
 *
 * ### :material-memory: Rotation limits
 *
 * Rotation starts after this writer accounts for 10 MiB and retains three files.
 * Engine writes are not included in that counter. A final rename failure keeps the
 * counter and retries on later writes, so 10 MiB is not a file-size limit.
 */
class MO2_API Logger
{
public:
    /**
     * @fn Logger& Logger::instance()
     * @brief Keeps console output available when file setup fails.
     * @author Alex (<https://github.com/lextpf>)
     *
     * File setup failures write to stderr and leave file output disabled. Console
     * and callback output remain available.
     *
     * @return The instance, valid until process exit.
     */
    static Logger& instance();

    /**
     * @fn void Logger::set_callback(LogCallback)
     * @brief Redirects file output instead of duplicating it.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A set callback replaces file output. An in-flight call can retain the prior
     * pointer after `set_callback` returns. The message pointer passed to the
     * callback is borrowed and is valid only for that invocation.
     *
     * @param callback Function pointer, or null to restore file output.
     * @warning The target must be thread-safe and outlive all in-flight log calls.
     */
    void set_callback(LogCallback callback);

    /**
     * @fn void Logger::log(const std::string&)
     * @brief Keeps console and callback latency outside the file lock.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Output order is file, stdout, then callback. The method takes the file mutex
     * even when callback routing suppresses the file write.
     *
     * @param message Raw message text.
     */
    void log(const std::string& message);

    /**
     * @fn void Logger::log_error(const std::string& message)
     * @brief Write an error record and send console output to stderr.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param message Raw text, also passed to the callback when one is set.
     */
    void log_error(const std::string& message);

    /**
     * @fn void Logger::log_warning(const std::string& message)
     * @brief Write a warning record and send console output to stdout.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param message Raw text, also passed to the callback when one is set.
     */
    void log_warning(const std::string& message);

    /**
     * @fn bool Logger::clear_log()
     * @brief Coordinates truncation with the persistent writer.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The mutex coordinates truncation with the persistent handle. Rotated files
     * remain. Failure can mean either truncation or reopen failed.
     *
     * @return `true` only after successful truncation and reopen.
     */
    bool clear_log();

    /**
     * @fn std::string Logger::log_path() const
     * @brief Report the active log path even when file output is unavailable.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return The module-relative logs/salma.log path as a native narrow string.
     */
    std::string log_path() const;

    /**
     * @fn std::string Logger::log_directory() const
     * @brief Anchors logs to the owning module instead of the working directory.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The directory is resolved from the module that owns `Logger`, not from the
     * host executable or working directory.
     *
     * @return The resolved directory as a native narrow string.
     */
    std::string log_directory() const;

private:
    /**
     * @fn Logger::Logger()
     * @brief Resolve the log directory and open the append stream.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Directory creation and open failures are reported to stderr. The byte counter starts from the
     * stream position when the file is available.
     */
    Logger();
    /**
     * @fn Logger::~Logger()
     * @brief Flush and close the append stream while containing cleanup exceptions.
     * @author Alex (<https://github.com/lextpf>)
     */
    ~Logger();
    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    /**
     * @fn void Logger::write_log_unlocked(const std::string&, const std::string&)
     * @brief Write one timestamped record, flush it, and check rotation.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param level Severity label stored in the file.
     * @param message Raw record text; a newline is appended.
     * @pre The caller holds mutex_.
     */
    void write_log_unlocked(const std::string& level, const std::string& message);

    /**
     * @fn void Logger::rotate_if_needed()
     * @brief Rotate after the writer byte count reaches 10 MiB.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A failed final rename reopens the current log and retains the byte count for a later attempt.
     *
     * @pre The caller holds mutex_.
     */
    void rotate_if_needed();

    // Rotation trigger, in bytes.
    static constexpr size_t kMaxLogSize = 10 * 1024 * 1024;
    // Retained rotated files.
    static constexpr int kMaxRotatedFiles = 3;

    /// External callback, or null for file output.
    std::atomic<LogCallback> callback_ = nullptr;
    std::string log_directory_;
    std::ofstream log_file_;
    // Serializes file writes, rotation, and truncation.
    std::mutex mutex_;
    /// Bytes observed or written by this logger. Engine writes are excluded.
    size_t bytes_written_ = 0;
};

}  // namespace mo2core

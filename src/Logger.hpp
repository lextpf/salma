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
 * @brief receives one raw UTF-8 log message.
 * @author Alex (https://github.com/lextpf)
 *
 * the string is valid only during the call. callbacks can run concurrently on
 * caller threads and must remain valid until all in-flight calls finish.
 *
 * @see Logger::set_callback
 */
using LogCallback = void (*)(const char*);

/**
 * @class Logger
 * @brief writes process logs to a callback or rotating file.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Logger
 *
 * ### :material-transit-connection-variant: output routing
 *
 * | callback | file output      | callback output |
 * |----------|------------------|-----------------|
 * | null     | `logs/salma.log` | none            |
 * | set      | none             | caller thread   |
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
 * ### :material-lock-outline: thread safety
 *
 * console output is always active. file writes are serialized. callback calls
 * can overlap and run outside the file mutex. callback exceptions are contained,
 * and same-thread recursive callbacks are dropped.
 *
 * the C++ and Rust loggers can append to the same file. each complete record,
 * including its newline, must use one write followed by a flush. splitting a
 * record permits the other writer to interleave bytes.
 *
 * ### :material-memory: rotation limits
 *
 * rotation starts after this writer accounts for 10 MiB and retains three files.
 * engine writes are not included in that counter. a final rename failure keeps the
 * counter and retries on later writes, so 10 MiB is not a file-size limit.
 */
class MO2_API Logger
{
public:
    /**
     * @fn Logger& Logger::instance()
     * @brief keeps console output available when file setup fails.
     * @author Alex (https://github.com/lextpf)
     *
     * file setup failures write to stderr and leave file output disabled. console
     * and callback output remain available.
     *
     * @return the instance, valid until process exit.
     */
    static Logger& instance();

    /**
     * @fn void Logger::set_callback(LogCallback)
     * @brief redirects file output instead of duplicating it.
     * @author Alex (https://github.com/lextpf)
     *
     * a set callback replaces file output. an in-flight call can retain the prior
     * pointer after `set_callback` returns.
     *
     * @param callback function pointer, or null to restore file output.
     * @warning the target must be thread-safe and outlive all in-flight log calls.
     */
    void set_callback(LogCallback callback);

    /**
     * @fn void Logger::log(const std::string&)
     * @brief keeps console and callback latency outside the file lock.
     * @author Alex (https://github.com/lextpf)
     *
     * output order is file, stdout, then callback. the method takes the file mutex
     * even when callback routing suppresses the file write.
     *
     * @param message raw message text.
     */
    void log(const std::string& message);

    void log_error(const std::string& message);

    void log_warning(const std::string& message);

    /**
     * @fn bool Logger::clear_log()
     * @brief coordinates truncation with the persistent writer.
     * @author Alex (https://github.com/lextpf)
     *
     * the mutex coordinates truncation with the persistent handle. rotated files
     * remain. failure can mean either truncation or reopen failed.
     *
     * @return `true` only after successful truncation and reopen.
     */
    bool clear_log();

    std::string log_path() const;

    /**
     * @fn std::string Logger::log_directory() const
     * @brief anchors logs to the owning module instead of the working directory.
     * @author Alex (https://github.com/lextpf)
     *
     * the directory is resolved from the module that owns `Logger`, not from the
     * host executable or working directory.
     *
     * @return the resolved directory as a native narrow string.
     */
    std::string log_directory() const;

private:
    Logger();
    ~Logger();
    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    // the caller holds mutex_ while writing.
    void write_log_unlocked(const std::string& level, const std::string& message);

    // the caller holds mutex_ while checking rotation.
    void rotate_if_needed();

    // rotation trigger, in bytes.
    static constexpr size_t kMaxLogSize = 10 * 1024 * 1024;
    // retained rotated files.
    static constexpr int kMaxRotatedFiles = 3;

    /// external callback, or null for file output.
    std::atomic<LogCallback> callback_ = nullptr;
    std::string log_directory_;
    std::ofstream log_file_;
    // guards file writes and callback reads.
    std::mutex mutex_;
    /// bytes observed or written by this logger. engine writes are excluded.
    size_t bytes_written_ = 0;
};

}  // namespace mo2core

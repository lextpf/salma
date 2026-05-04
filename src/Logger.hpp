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
 * @brief Function pointer type for external log callbacks.
 * @ingroup Logger
 *
 * The callback receives a null-terminated UTF-8 string holding the raw message.
 * Timestamp and level prefix go to file output only, never to the callback. A
 * message holding an interior NUL arrives truncated at that byte, while the file
 * line keeps it in full. The string is valid only for the duration of the call;
 * copy anything you keep.
 *
 * Logger invokes it on whichever thread called a log method, and several
 * threads may be inside it at once, so an implementation must be thread-safe.
 *
 * The pointer is stored and invoked as-is. Logger never copies, frees or
 * otherwise owns the target; the registrar keeps it alive.
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
 * Meyer's singleton with three levels (info, warning, error) and two sinks: an
 * optional callback for an in-process host, and `logs/salma.log` when no
 * callback is registered.
 *
 * ## :material-message-text-outline: Output Routing
 *
 * | Callback? |    Console    |   File    | Callback |
 * |-----------|---------------|-----------|----------|
 * |        No | stdout/stderr | salma.log | -        |
 * |       Yes | stdout/stderr |     -     | invoked  |
 *
 * Registering a callback through set_callback() forwards every message to it
 * and skips the file write, on the assumption that the host handles
 * persistence. Console output (stdout for info and warning, stderr for error)
 * is always active either way.
 *
 * Nothing in `mo2-server` or `salma_tests` calls set_callback(), so the shipped
 * binaries always take the file branch and the callback column is a supported
 * path nothing currently uses. The MO2 Python plugin does register a callback,
 * but with the engine DLL (`setLogCallback` in `src/capi.rs`, driving
 * `src/logger.rs`), not with this class.
 *
 * ## :material-format-text: Log Format
 *
 * File entries are formatted as:
 * ```
 * 2026-02-28 19:45:02.123 INFO [install] message text
 * ```
 *
 * The trailing newline is part of that string, not a second write. The Shared
 * File section below explains why that matters.
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
 *
 * `mutex_` guards two things together: the file write, and the read of
 * `callback_` that decides whether a file write happens at all. Every log
 * method therefore takes it, even when a callback is registered and no file
 * write follows. `callback_` is a `std::atomic<LogCallback>` on the default
 * sequentially consistent ordering, which keeps set_callback() itself lock-free
 * but does not make the log methods lock-free.
 *
 * Console output (`std::cout` / `std::cerr`) is not mutex-guarded, so
 * concurrent log calls can interleave lines on the terminal. File output is
 * serialized by `mutex_`. Callback dispatch is not serialized: each call sees
 * one consistent pointer, but several threads may be inside the callback at
 * once, so the callback must be thread-safe.
 *
 * **Callback failure.** Every invocation is wrapped in `catch (...)`. A
 * throwing callback produces a diagnostic on stderr and nothing else; the
 * exception never reaches the log site.
 *
 * **Reentrancy.** The callback pointer is read under `mutex_` but invoked
 * outside the lock, so calling a Logger method from inside a callback does not
 * deadlock. Self-recursion on the same thread is caught by a `thread_local`
 * flag: the inner call writes one `[Logger] Re-entrant callback dropped: ...`
 * line to stderr and skips the dispatch. Console output still happens; the file
 * write does not, because reaching that path means a callback is registered, and
 * a registered callback replaces the file write. This protects the process from
 * stack overflow without every callback implementer adding a guard.
 *
 * ## :material-source-branch: Shared File
 *
 * `logs/salma.log` has two independent writers: this class, and the engine's
 * logger in `src/logger.rs`. The engine DLL is deployed next to
 * `mo2-server.exe`, so both resolve the same absolute path. Both open in append
 * mode, so neither can overwrite the other, but append mode only makes an
 * individual write atomic. It cannot glue two writes into one record.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * sequenceDiagram
 *     participant C as mo2-server Logger
 *     participant F as logs/salma.log
 *     participant R as mo2-salma.dll logger
 *     C->>F: write one complete line, newline included
 *     C->>F: flush
 *     R->>F: write one complete line, newline included
 *     Note over C,R: both handles are in append mode
 *     Note over C,R: the buffer is empty between records, so every
 *     Note over C,R: write the OS sees is exactly one whole line
 * ```
 *
 * That makes the single-write rule a contract, not an implementation detail:
 * **emit each record with one `write` call and flush immediately.** Inserting
 * the line and the newline separately into a buffered stream lets the buffer
 * drain at whatever byte fills it, routinely mid-line, and the engine's next
 * append then lands inside the half-written record. A sample log torn that way
 * had 129 of 11,547 lines damaged, each one a bogus record in the dashboard's
 * log parser. Do not split the write in two, and do not drop the flush.
 *
 * A second consequence: `bytes_written_` counts the file size observed when
 * this process opened the file, plus what this writer has appended since. Bytes
 * the engine appends are not counted, so the rotation trigger below fires on
 * this writer's own volume and the file on disk can be larger than the trigger
 * value.
 *
 * A third: rotation is not coordinated between the two writers. Neither closes
 * or reopens the other's handle, so a rename by one leaves the other appending
 * through a handle that no longer points at `salma.log`, and a rename attempted
 * while the other writer still holds the file open can fail outright. That is
 * the second case in Rotation Failure Behavior below.
 *
 * ## :material-autorenew: Log Rotation
 *
 * Rotation is checked after every file write and runs once `bytes_written_`
 * reaches 10 MiB. Up to 3 rotated files are kept (`salma.log.1` through
 * `salma.log.3`). Rotation closes the active file, shifts the rotated files
 * (delete `.3`, `.2` becomes `.3`, `.1` becomes `.2`), renames `salma.log` to
 * `salma.log.1`, and reopens a fresh `salma.log`.
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * stateDiagram-v2
 *     [*] --> active: open salma.log in append mode
 *     active --> shifting: bytes_written_ >= 10 MiB, close the file
 *     shifting --> renaming: delete .3, rename .2 to .3 and .1 to .2
 *     shifting --> renaming: a shift step failed, error_code to stderr, ignored
 *     renaming --> active: rename salma.log to .1 ok, reopen fresh, bytes_written_ = 0
 *     renaming --> active: rename failed, error_code to stderr, reopen, bytes_written_ kept
 * ```
 *
 * ## :material-alert-circle-outline: Rotation Failure Behavior
 *
 * The two rotation stages fail differently, and the difference is load-bearing:
 *
 * - **A shift step fails** (removing `salma.log.3`, or renaming `.1` to `.2` or
 *   `.2` to `.3`). The `std::error_code` message goes to `stderr` and the
 *   failure is then ignored; rotation continues. A rotated file that cannot be
 *   renamed away is overwritten by the next successful rename of the file below
 *   it.
 * - **The final `salma.log` to `salma.log.1` rename fails** (on Windows,
 *   typically a sharing violation from the engine's own handle on the same
 *   file, an antivirus scanner or a log viewer holding the file open). The
 *   message goes to `stderr`, the existing `salma.log` is reopened in append
 *   mode, and this rotation attempt is abandoned. `bytes_written_` is
 *   deliberately left alone: resetting it would hide the condition. The active
 *   file keeps growing past 10 MiB and rotation is retried on every following
 *   write until a rename succeeds.
 *
 * The 10 MiB figure is therefore a rotation trigger, not a cap on the size of
 * `salma.log`.
 *
 * @see mo2server::SalmaEngine for the server's path to the engine DLL,
 *   whose own `setLogCallback` export owns the host-callback path.
 */
class MO2_API Logger
{
public:
    /**
     * @brief Get the singleton Logger instance.
     *
     * The first call constructs the instance and creates the `logs/` directory.
     * Thread-safe via the magic static.
     *
     * Construction never fails. If `logs/` cannot be created or `salma.log`
     * cannot be opened, a `[Logger]` diagnostic goes to stderr and the instance
     * is still returned holding no file handle. In that state every file write
     * is dropped silently while console and callback output continue, and no
     * accessor reports it; a later successful clear_log() reopens the handle
     * and ends the state.
     */
    static Logger& instance();

    /**
     * @brief Register an external log callback.
     *
     * While a callback is set, every message is forwarded to it and file
     * logging is disabled. Pass `nullptr` to revert to file logging.
     *
     * The store is a lock-free atomic write, so this never blocks on a logging
     * thread. A log call already in flight may still use the previous pointer.
     *
     * @warning The target must stay alive and thread-safe for as long as any
     * log call can reach it. Clearing does not wait for an in-flight
     * invocation, because the callback is invoked outside the lock, so the
     * target must outlive every log call that snapshotted it. A dangling
     * function pointer is undefined behavior.
     *
     * @param callback Function pointer, or `nullptr` to clear.
     */
    void set_callback(LogCallback callback);

    /**
     * @brief Log an informational message.
     *
     * Output order: file (under `mutex_`, only when no callback is registered),
     * then stdout, then the callback. Console and callback run outside the lock
     * so slow terminal I/O and slow callbacks do not hold back the file write.
     *
     * **Blocking:** takes `mutex_` in every case, even when a callback is
     * registered and no file write follows, because the callback pointer is
     * read under the same lock.
     *
     * @param message Message text, conventionally prefixed with a subsystem tag
     *        such as `[install]`.
     */
    void log(const std::string& message);

    /**
     * @brief Log an error message.
     *
     * As log(), except the console half goes to stderr. `mutex_` is taken in
     * every case.
     *
     * @param message Message text.
     */
    void log_error(const std::string& message);

    /**
     * @brief Log a warning message.
     *
     * Same routing as log(), including the stdout half. `mutex_` is taken in
     * every case.
     *
     * @param message Message text.
     */
    void log_warning(const std::string& message);

    /**
     * @brief Truncate the log file safely.
     *
     * Takes `mutex_`, closes the persistent handle, truncates, and reopens in
     * append mode. Truncating from outside while Logger holds an open handle
     * corrupts the file; use this instead.
     *
     * **Failure modes.** A `false` return covers two states the caller cannot
     * tell apart from the return value alone:
     *
     * - Truncation failed, so the file still holds its contents. The failure is
     *   detected on the truncating open itself. The handle is reopened in
     *   append mode (the reopen result is not checked on this path) and
     *   `bytes_written_` is deliberately kept, because nothing was removed from
     *   the file.
     * - Truncation succeeded and the reopen failed. There is no open handle,
     *   `bytes_written_` is 0, and every later file write is silently dropped.
     *   Console and callback output continue, and a later successful
     *   clear_log() reopens the handle and ends the state.
     *
     * @return `true` only when the file was both truncated and reopened.
     */
    bool clear_log();

    /**
     * @brief Return the path to the active log file.
     * @throw Does not throw.
     */
    std::string log_path() const;

    /**
     * @brief Return the path to the active log directory.
     *
     * Resolved once at construction by passing `&Logger::instance` into
     * `mo2core::module_directory(...)`, which walks back from that address to
     * the module containing it. `logs/` therefore always sits next to the
     * binary that owns this code, whatever the host process's working directory
     * or the host executable's location. Anything deriving the log path from
     * `current_path()` will disagree with where Logger actually writes; use
     * this accessor.
     *
     * Falls back to `current_path()` only when the platform lookup fails, for
     * example on a non-Windows build.
     *
     * @throw Does not throw.
     */
    std::string log_directory() const;

private:
    Logger();
    ~Logger();
    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    // Writes to the log file. Caller must hold mutex_.
    void write_log_unlocked(const std::string& level, const std::string& message);

    // Rotates the log file once bytes_written_ reaches kMaxLogSize.
    // Caller must hold mutex_.
    void rotate_if_needed();

    static constexpr size_t kMaxLogSize = 10 * 1024 * 1024;  ///< Rotation trigger, in bytes.
    static constexpr int kMaxRotatedFiles = 3;               ///< Keep salma.log.1 through .3.

    /// External callback, or null to use file logging. Read under mutex_
    /// together with the file-write decision; stored lock-free.
    std::atomic<LogCallback> callback_ = nullptr;
    std::string log_directory_;  ///< Absolute path to the logs directory.
    std::ofstream log_file_;     ///< Persistent log file handle (append mode).
    std::mutex mutex_;           ///< Guards file writes and the callback_ read.
    /// Bytes this writer accounts for: the file size seen at open, plus every
    /// byte written since. Bytes the Rust engine appends are not counted.
    size_t bytes_written_ = 0;
};

}  // namespace mo2core

// Logger - the server's process-wide log sink, a Meyers singleton created on
// first use and destroyed at process exit.
//
// Location. The log directory is <module dir>/logs, resolved once in the
// constructor from the address of Logger::instance rather than from the working
// directory. In mo2-server.exe that is the exe directory. The engine DLL has
// its own logger (src/logger.rs) that resolves the same way, so when the DLL
// sits next to the exe both halves write the same file. write_log_unlocked
// explains what that costs and why every record is one write plus a flush.
//
// Output routing. Each log call takes one of two paths, never both:
//
//   log / log_warning / log_error
//     |
//     +-- under mutex_: snapshot callback_
//     |     no callback  -> write_log_unlocked() -> file, then rotate check
//     |     callback set -> nothing is written to the file
//     |
//     +-- outside mutex_: console (stdout for info and warning, stderr for
//     |   error), always, so slow console I/O never holds the lock
//     |
//     +-- outside mutex_: the callback, if one was set, guarded against
//           re-entry per thread and against exceptions
//
// Setting a callback redirects file output; it does not tee it. A host that
// installs a callback owns persistence from that point on.
//
// Rotation. write_log_unlocked counts bytes and calls rotate_if_needed at
// kMaxLogSize (10 MiB): salma.log.3 is deleted, .2 becomes .3, .1 becomes .2,
// and salma.log becomes .1. A failed rename leaves the byte counter alone and
// the current file keeps growing; rotate_if_needed explains why.
//
// Thread safety. Every public method is safe to call from any thread. mutex_
// serializes the file handle and the byte counter; callback_ is atomic, so the
// snapshot needs no lock of its own.

#include "Logger.hpp"
#include "Utils.hpp"

#include <chrono>
#include <filesystem>
#include <format>
#include <fstream>
#include <iostream>

namespace fs = std::filesystem;

namespace mo2core
{

namespace
{

// Re-entrancy guard for the host-supplied log callback. A callback that itself
// calls Logger::log* would otherwise recurse until the stack overflows.
// Per-thread, because a callback runs on whichever thread logged.
thread_local bool g_in_callback = false;

struct CallbackReentryGuard
{
    CallbackReentryGuard() { g_in_callback = true; }
    ~CallbackReentryGuard() { g_in_callback = false; }
    CallbackReentryGuard(const CallbackReentryGuard&) = delete;
    CallbackReentryGuard& operator=(const CallbackReentryGuard&) = delete;
};

}  // namespace

Logger& Logger::instance()
{
    static Logger inst;
    return inst;
}

Logger::Logger()
{
    // Anchor logs/ to the module that owns this code, so the log path does not
    // depend on the host process's working directory: mo2-server.exe and
    // salma_tests both write <module dir>/logs/salma.log. The MO2 Python plugin
    // never reaches this class; it drives the engine DLL's logger.
    // module_directory() falls back to the working directory if the platform
    // lookup fails.
    auto module_dir = module_directory(reinterpret_cast<const void*>(&Logger::instance));
    log_directory_ = (module_dir / "logs").string();

    std::error_code ec;
    fs::create_directories(log_directory_, ec);
    if (ec)
    {
        std::cerr << "[Logger] Failed to create log directory " << log_directory_ << ": "
                  << ec.message() << std::endl;
    }

    log_file_.open(fs::path(log_directory_) / "salma.log", std::ios::app);
    if (log_file_.is_open())
    {
        auto pos = log_file_.tellp();
        bytes_written_ = (pos > 0) ? static_cast<size_t>(pos) : 0;
    }
    else
    {
        std::cerr << "[Logger] Failed to open log file: "
                  << (fs::path(log_directory_) / "salma.log").string() << std::endl;
    }
}

Logger::~Logger()
{
    try
    {
        if (log_file_.is_open())
        {
            log_file_.flush();
            log_file_.close();
        }
    }
    catch (...)
    {
    }
}

void Logger::set_callback(LogCallback callback)
{
    callback_.store(callback);
}

// The three log methods below share one shape, drawn in the routing diagram at
// the top of this file. They differ only in the level string and in whether the
// console copy goes to stdout or stderr. Two details the diagram cannot show:
//   - The callback is snapshotted under mutex_ but called outside it, so a
//     callback that logs cannot deadlock on the logger's own mutex.
//   - Callback exceptions are swallowed and reported to stderr. A log call must
//     never propagate a failure into a call site that was only logging.

void Logger::log(const std::string& message)
{
    LogCallback cb_snapshot = nullptr;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        cb_snapshot = callback_.load();
        if (!cb_snapshot)
        {
            write_log_unlocked("INFO", message);
        }
    }
    // Console output stays outside the lock; interleaving there is acceptable.
    std::cout << message << '\n';
    if (cb_snapshot)
    {
        if (g_in_callback)
        {
            std::cerr << "[Logger] Re-entrant callback dropped: " << message << '\n';
        }
        else
        {
            CallbackReentryGuard guard;
            try
            {
                cb_snapshot(message.c_str());
            }
            catch (...)
            {
                std::cerr << "[Logger] Callback threw for: " << message << '\n';
            }
        }
    }
}

void Logger::log_error(const std::string& message)
{
    LogCallback cb_snapshot = nullptr;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        cb_snapshot = callback_.load();
        if (!cb_snapshot)
        {
            write_log_unlocked("ERROR", message);
        }
    }
    std::cerr << message << '\n';
    if (cb_snapshot)
    {
        if (g_in_callback)
        {
            std::cerr << "[Logger] Re-entrant callback dropped: " << message << '\n';
        }
        else
        {
            CallbackReentryGuard guard;
            try
            {
                cb_snapshot(message.c_str());
            }
            catch (...)
            {
                std::cerr << "[Logger] Callback threw for: " << message << '\n';
            }
        }
    }
}

void Logger::log_warning(const std::string& message)
{
    LogCallback cb_snapshot = nullptr;
    {
        std::lock_guard<std::mutex> lock(mutex_);
        cb_snapshot = callback_.load();
        if (!cb_snapshot)
        {
            write_log_unlocked("WARNING", message);
        }
    }
    std::cout << message << '\n';
    if (cb_snapshot)
    {
        if (g_in_callback)
        {
            std::cerr << "[Logger] Re-entrant callback dropped: " << message << '\n';
        }
        else
        {
            CallbackReentryGuard guard;
            try
            {
                cb_snapshot(message.c_str());
            }
            catch (...)
            {
                std::cerr << "[Logger] Callback threw for: " << message << '\n';
            }
        }
    }
}

bool Logger::clear_log()
{
    std::lock_guard<std::mutex> lock(mutex_);
    auto path = fs::path(log_directory_) / "salma.log";

    if (log_file_.is_open())
    {
        log_file_.flush();
        log_file_.close();
    }

    {
        std::ofstream ofs(path, std::ios::trunc);
        if (!ofs)
        {
            // Reopen for append even when the truncate failed, so later log
            // calls still reach the file.
            log_file_.open(path, std::ios::app);
            return false;
        }
    }

    log_file_.open(path, std::ios::app);
    bytes_written_ = 0;
    return log_file_.is_open();
}

std::string Logger::log_path() const
{
    return (fs::path(log_directory_) / "salma.log").string();
}

std::string Logger::log_directory() const
{
    return log_directory_;
}

void Logger::write_log_unlocked(const std::string& level, const std::string& message)
{
    // Caller must hold mutex_.
    if (!log_file_.is_open())
        return;

    auto now = std::chrono::system_clock::now();
    auto time_t_now = std::chrono::system_clock::to_time_t(now);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(now.time_since_epoch()) % 1000;
    std::tm tm_now{};
#ifdef _WIN32
    localtime_s(&tm_now, &time_t_now);
#else
    localtime_r(&time_t_now, &tm_now);
#endif

    // Format: "YYYY-MM-DD HH:MM:SS.mmm LEVEL message\n". The newline is part of
    // the string, not a second insertion; the next comment explains why.
    auto line = std::format("{:04d}-{:02d}-{:02d} {:02d}:{:02d}:{:02d}.{:03d} {} {}\n",
                            tm_now.tm_year + 1900,
                            tm_now.tm_mon + 1,
                            tm_now.tm_mday,
                            tm_now.tm_hour,
                            tm_now.tm_min,
                            tm_now.tm_sec,
                            static_cast<int>(ms.count()),
                            level,
                            message);

    // One write, then a flush, with the newline inside `line`.
    //
    // salma.log has two independent writers: this logger and the Rust engine's
    // (src/logger.rs), which appends through its own handle to the same path,
    // because the DLL is deployed next to the exe. Both open O_APPEND, so
    // neither can overwrite the other, but O_APPEND only makes an individual
    // write atomic; it cannot glue two writes together.
    //
    // A buffered `log_file_ << line << '\n'` with no flush empties the buffer at
    // whatever byte it happens to fill on, routinely mid-line. The engine's next
    // append then lands inside this record and produces a torn pair, such as
    // "20262026-08-12 ... [archive] ..." followed by the orphaned remainder
    // "-08-12 ... [crow] ...". A sample log carried 129 damaged lines out of
    // 11,547, and the dashboard's parser turned each one into a bogus record
    // with a timestamp where a subsystem tag belongs.
    //
    //   buffered, no flush   [2026-08-12 10:00:01.123 INFO [crow] Req]
    //                                                             ^ buffer
    //                                                               emptied
    //                                                               mid-record
    //   engine appends                                            [2026-08-12 10:00:01.124 ...
    //   on disk              2026-08-12 10:00:01.123 INFO [crow] Req2026-08-12 ... [archive] ...
    //                        ues: GET /api/logs ...   <- orphaned remainder, its
    //                                                    own "line" to the parser
    //
    //   one write + flush    [2026-08-12 10:00:01.123 INFO [crow] Request: ...\n]
    //                        every write the OS sees is exactly one complete record
    //
    // Writing the whole line in one call and flushing immediately leaves the
    // buffer empty between records. Do not split this back into two insertions,
    // and do not drop the flush.
    log_file_.write(line.data(), static_cast<std::streamsize>(line.size()));
    log_file_.flush();
    bytes_written_ += line.size();
    rotate_if_needed();
}

void Logger::rotate_if_needed()
{
    // Caller must hold mutex_.
    if (bytes_written_ < kMaxLogSize)
        return;

    log_file_.flush();
    log_file_.close();

    auto log_dir = fs::path(log_directory_);

    // Shift existing rotated files: .3 deleted, .2 -> .3, .1 -> .2
    for (int i = kMaxRotatedFiles; i >= 1; --i)
    {
        auto src = log_dir / std::format("salma.log.{}", i);
        if (!fs::exists(src))
            continue;
        if (i == kMaxRotatedFiles)
        {
            std::error_code ec;
            fs::remove(src, ec);
            if (ec)
            {
                std::cerr << "[Logger] Failed to remove rotated log " << src.string() << ": "
                          << ec.message() << std::endl;
            }
        }
        else
        {
            auto dst = log_dir / std::format("salma.log.{}", i + 1);
            std::error_code ec;
            fs::rename(src, dst, ec);
            if (ec)
            {
                std::cerr << "[Logger] Failed to rename " << src.string() << " -> " << dst.string()
                          << ": " << ec.message() << std::endl;
            }
        }
    }

    // Rotate the current log to .1.
    //
    // On Windows, fs::rename of an open file fails with ERROR_SHARING_VIOLATION.
    // log_file_ is closed above, but the file can still be pinned by an
    // antivirus scanner, a tail-style log viewer or a transient explorer.exe
    // handle. On that failure bytes_written_ is left alone: the file is reopened
    // for append and grows past kMaxLogSize until a later rotation succeeds.
    // That beats dropping log entries, and the rename error goes to stderr so an
    // operator can investigate.
    {
        std::error_code ec;
        fs::rename(log_dir / "salma.log", log_dir / "salma.log.1", ec);
        if (ec)
        {
            std::cerr << "[Logger] Failed to rotate salma.log -> salma.log.1: " << ec.message()
                      << std::endl;
            log_file_.open(log_dir / "salma.log", std::ios::app);
            return;
        }
    }

    log_file_.open(log_dir / "salma.log", std::ios::app);
    bytes_written_ = 0;
}

}  // namespace mo2core

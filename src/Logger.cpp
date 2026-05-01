#include "Logger.h"
#include "Utils.h"

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

// Re-entrancy guard for the user-supplied log callback. A callback that
// itself calls Logger::log* would otherwise recurse until the stack
// overflows. Per-thread because callbacks may run on any caller's thread.
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
    // Anchor logs/ next to the module that owns this code (the mo2-salma DLL).
    // This makes the log path deterministic regardless of the host process's
    // working directory: mo2-server.exe and the MO2 Python plugin both end up
    // writing into <module_dir>/logs/salma.log. module_directory() falls back
    // to cwd if the platform lookup fails, preserving the previous backstop.
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

// Output routing for all three log methods:
//   1. Under mutex_: snapshot callback_ (atomic load); if no callback, write to file.
//   2. Outside mutex_: write to console (stdout for info/warning, stderr for error).
//   3. If callback was set: forward to callback (no file write).
//      Callback exceptions are caught to prevent crashing log call sites.
//   Console output is outside the lock to avoid holding the mutex during slow I/O.

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
    // Console output outside the lock -- interleaving is acceptable.
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

    // Truncate the file
    {
        std::ofstream ofs(path, std::ios::trunc);
        if (!ofs)
        {
            // Reopen in append mode even on failure
            log_file_.open(path, std::ios::app);
            return false;
        }
    }

    // Reopen in append mode
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

    // Format: "YYYY-MM-DD HH:MM:SS.mmm LEVEL message"
    auto line = std::format("{:04d}-{:02d}-{:02d} {:02d}:{:02d}:{:02d}.{:03d} {} {}",
                            tm_now.tm_year + 1900,
                            tm_now.tm_mon + 1,
                            tm_now.tm_mday,
                            tm_now.tm_hour,
                            tm_now.tm_min,
                            tm_now.tm_sec,
                            static_cast<int>(ms.count()),
                            level,
                            message);
    log_file_ << line << '\n';
    bytes_written_ += line.size() + 1;  // +1 for newline
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

    // Rotate current log to .1
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

    // Open a fresh log file
    log_file_.open(log_dir / "salma.log", std::ios::app);
    bytes_written_ = 0;
}

}  // namespace mo2core

#include "Logger.h"
#include <chrono>
#include <filesystem>
#include <format>
#include <fstream>
#include <iostream>

namespace fs = std::filesystem;

namespace mo2core
{

Logger& Logger::instance()
{
    static Logger inst;
    return inst;
}

Logger::Logger()
{
    log_directory_ = (fs::current_path() / "logs").string();
    fs::create_directories(log_directory_);
    log_file_.open(fs::path(log_directory_) / "salma.log", std::ios::app);
}

Logger::~Logger()
{
    if (log_file_.is_open())
    {
        log_file_.flush();
        log_file_.close();
    }
}

void Logger::set_callback(LogCallback callback)
{
    callback_.store(callback, std::memory_order_release);
}

// Output routing for all three log methods:
//   1. Snapshot callback_ once (atomic load) for consistent routing.
//   2. If callback is set -> forward to callback (no file write).
//   3. Always write to console (stdout for info/warning, stderr for error).
//   4. If no callback -> append to logs/salma.log via write_log().

void Logger::log(const std::string& message)
{
    auto cb = callback_.load(std::memory_order_acquire);
    if (cb)
    {
        cb(message.c_str());
    }
    std::cout << message << std::endl;
    if (!cb)
    {
        write_log("INFO", message);
    }
}

void Logger::log_error(const std::string& message)
{
    auto cb = callback_.load(std::memory_order_acquire);
    if (cb)
    {
        cb(message.c_str());
    }
    std::cerr << message << std::endl;
    if (!cb)
    {
        write_log("ERROR", message);
    }
}

void Logger::log_warning(const std::string& message)
{
    auto cb = callback_.load(std::memory_order_acquire);
    if (cb)
    {
        cb(message.c_str());
    }
    std::cout << message << std::endl;
    if (!cb)
    {
        write_log("WARNING", message);
    }
}

void Logger::write_log(const std::string& level, const std::string& message)
{
    // Mutex-guarded: serializes concurrent file writes.
    std::lock_guard<std::mutex> lock(mutex_);

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
    log_file_ << std::format("{:04d}-{:02d}-{:02d} {:02d}:{:02d}:{:02d}.{:03d} {} {}",
                             tm_now.tm_year + 1900,
                             tm_now.tm_mon + 1,
                             tm_now.tm_mday,
                             tm_now.tm_hour,
                             tm_now.tm_min,
                             tm_now.tm_sec,
                             static_cast<int>(ms.count()),
                             level,
                             message)
              << std::endl;
}

}  // namespace mo2core

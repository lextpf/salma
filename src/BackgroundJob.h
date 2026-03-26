#pragma once

#include <atomic>
#include <condition_variable>
#include <functional>
#include <iostream>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace mo2server
{

/**
 * @class BackgroundJob
 * @brief Generic async job runner with result storage and thread lifecycle management.
 * @ingroup Core
 *
 * Encapsulates the recurring pattern of: atomic running flag, mutex-guarded
 * result, and a joinable background thread. Callers provide a work function
 * that returns a `TResult`; the template handles start gating, thread join,
 * exception capture, and result access.
 *
 * @tparam TResult The result type stored on completion.
 *
 * ## Cooperative Cancellation
 *
 * BackgroundJob exposes a cancellation token (`cancel_token()`) that work
 * functions can poll via `is_cancel_requested()`. The destructor sets the
 * token and waits up to 10 seconds for the work function to exit before
 * blocking on `thread::join()`. Work functions that perform long-running
 * loops should check the token periodically.
 *
 * @warning The destructor blocks until the background thread completes.
 * Detaching is unsafe because the lambda captures `this`.
 */
template <typename TResult>
class BackgroundJob
{
    static_assert(std::is_move_constructible_v<TResult>,
                  "BackgroundJob requires TResult to be move-constructible");

public:
    BackgroundJob() = default;

    ~BackgroundJob()
    {
        try
        {
            cancel_requested_.store(true);
            std::unique_lock<std::mutex> lock(mutex_);
            if (thread_.joinable())
            {
                if (running_.load())
                {
                    // Wait up to 10s for the work function to notice the cancellation.
                    bool finished = cv_.wait_for(
                        lock, std::chrono::seconds(10), [this]() { return !running_.load(); });
                    if (!finished)
                    {
                        // Use stderr directly -- Logger singleton may already be destroyed.
                        std::cerr << "[BackgroundJob] Destructor: work function did not stop "
                                     "within 10s, blocking on join\n";
                    }
                }
                // Unlock before join to avoid deadlock with the lambda that acquires mutex_.
                lock.unlock();
                thread_.join();
            }
        }
        catch (...)
        {
            // Swallow all exceptions — destructors must not throw (MISRA 15-5-1).
        }
    }
    BackgroundJob(const BackgroundJob&) = delete;
    BackgroundJob& operator=(const BackgroundJob&) = delete;

    /**
     * @brief Attempt to launch a background job.
     *
     * Returns `false` if a job is already running (concurrent start rejected).
     * The work function is called on a new thread; its return value is stored
     * as the result. Exceptions are caught and stored as an error string.
     *
     * @param work Callable returning `TResult`. Invoked on the background thread.
     * @return `true` if the job was started, `false` if one is already running.
     */
    bool try_start(std::function<TResult()> work)
    {
        std::unique_lock<std::mutex> lock(mutex_);

        // Check-and-set under the same mutex that the thread lambda uses to
        // clear running_.  This closes the race window where a previous
        // thread's store(false) + notify could interleave with exchange(true).
        if (running_.load())
            return false;
        running_.store(true);

        result_.reset();
        last_error_.clear();
        cancel_requested_.store(false);

        if (thread_.joinable())
        {
            // Previous thread already completed (running_ was false to reach here).
            // Unlock before join to avoid deadlock with the lambda.
            lock.unlock();
            thread_.join();
            lock.lock();
        }

        try
        {
            thread_ = std::thread(
                [this, work = std::move(work)]()
                {
                    try
                    {
                        auto result = work();
                        std::lock_guard<std::mutex> lk(mutex_);
                        result_ = std::move(result);
                        last_error_.clear();
                        running_.store(false);
                        cv_.notify_all();
                    }
                    catch (const std::exception& ex)
                    {
                        std::lock_guard<std::mutex> lk(mutex_);
                        last_error_ = ex.what();
                        running_.store(false);
                        cv_.notify_all();
                    }
                    catch (...)
                    {
                        std::lock_guard<std::mutex> lk(mutex_);
                        last_error_ = "Unknown error";
                        running_.store(false);
                        cv_.notify_all();
                    }
                });
        }
        catch (...)
        {
            running_.store(false);
            throw;
        }

        return true;
    }

    /// Check if the job is currently running (lock-free).
    [[nodiscard]] bool is_running() const { return running_.load(); }

    /// Request cooperative cancellation. Work functions should check `is_cancel_requested()`.
    void request_cancel() { cancel_requested_.store(true); }

    /// Check if cancellation has been requested (lock-free).
    [[nodiscard]] bool is_cancel_requested() const { return cancel_requested_.load(); }

    /// Return a reference to the cancellation token for passing to work functions.
    [[nodiscard]] const std::atomic<bool>& cancel_token() const { return cancel_requested_; }

    /**
     * @brief Read the job result under the mutex.
     *
     * The callback receives `(has_result, result, error)` while the mutex
     * is held, so the caller can safely inspect all fields. Returns
     * whatever the callback returns.
     *
     * @param fn Callable with signature `auto(bool has_result, const TResult* result, const
     * std::string& error)`. The pointer is null when `has_result` is false.
     */
    template <typename Fn>
    auto read_result(Fn&& fn) const
    {
        std::lock_guard<std::mutex> lock(mutex_);
        // Use a pointer to avoid requiring TResult to be default-constructible.
        const TResult* ptr = result_.has_value() ? &*result_ : nullptr;
        return fn(result_.has_value(), ptr, last_error_);
    }

private:
    std::atomic<bool> running_{false};
    std::atomic<bool> cancel_requested_{false};
    mutable std::mutex mutex_;
    std::condition_variable cv_;
    std::optional<TResult> result_;
    std::string last_error_;
    std::thread thread_;
};

}  // namespace mo2server

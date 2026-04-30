#pragma once

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <functional>
#include <iostream>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace mo2server
{

/**
 * @class BackgroundJob
 * @brief Generic async job runner with result storage and thread lifecycle management.
 * @author Alex (https://github.com/lextpf)
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
 * token, waits up to the grace period for the work function to exit, and
 * then **detaches** the thread (so a non-cooperative worker does not
 * indefinitely block process shutdown). Detach is safe because all state
 * accessed by the worker lives in a heap-allocated `State` struct held by
 * a `shared_ptr` that the worker captures by value -- the state outlives
 * `*this` if the worker is still running at destruction time.
 *
 * Work functions that perform long-running loops should still check the
 * token periodically; detach leaks the thread and any resources it holds
 * until the work eventually returns.
 *
 * ## Lifecycle
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * stateDiagram-v2
 *     [*] --> idle
 *     idle --> running: try_start() returns true
 *     idle --> idle: try_start() returns false (already running)
 *     running --> idle: work() returns - result_ set
 *     running --> idle: work() throws - last_error_ set
 *     running --> cancelling: request_cancel()
 *     cancelling --> idle: work() observes token & exits
 * ```
 */
template <typename TResult>
class BackgroundJob
{
    static_assert(std::is_move_constructible_v<TResult>,
                  "BackgroundJob requires TResult to be move-constructible");

    // All worker-visible state lives in a heap-allocated struct held by a
    // shared_ptr. The worker captures the shared_ptr by value, which keeps
    // the state alive even after the BackgroundJob owner has been destroyed.
    // This is what makes detach-on-shutdown safe.
    struct State
    {
        std::atomic<bool> running{false};
        std::atomic<bool> cancel_requested{false};
        mutable std::mutex mutex;
        std::condition_variable cv;
        std::optional<TResult> result;
        std::string last_error;
    };

public:
    /// Grace period the destructor waits for a cooperative shutdown before
    /// detaching the worker thread.
    static constexpr std::chrono::seconds kShutdownGrace{10};

    BackgroundJob()
        : state_(std::make_shared<State>())
    {
    }

    ~BackgroundJob() { shutdown(kShutdownGrace); }

    BackgroundJob(const BackgroundJob&) = delete;
    BackgroundJob& operator=(const BackgroundJob&) = delete;

    /**
     * @brief Cooperatively shut down the running job and reap the thread.
     *
     * Sets the cancellation flag, waits up to @p grace for the worker to
     * observe it and exit, then either joins (if exited cleanly) or
     * detaches (if still running). Detaching is safe because the worker
     * captures `state_` by value, so the State outlives `*this`.
     *
     * Idempotent and safe to call multiple times. Called automatically
     * from the destructor.
     */
    void shutdown(std::chrono::milliseconds grace =
                      std::chrono::duration_cast<std::chrono::milliseconds>(kShutdownGrace))
    {
        try
        {
            state_->cancel_requested.store(true);
            std::unique_lock<std::mutex> lock(state_->mutex);
            if (!thread_.joinable())
            {
                return;
            }
            if (state_->running.load())
            {
                const bool finished =
                    state_->cv.wait_for(lock, grace, [this]() { return !state_->running.load(); });
                lock.unlock();
                if (!finished)
                {
                    // The worker did not observe cancellation in time. Detach
                    // and warn loudly. The State stays alive for as long as the
                    // worker holds its shared_ptr; we just leak the std::thread
                    // handle and any resources the work captures.
                    std::cerr << "[BackgroundJob] Worker did not stop within grace period; "
                                 "detaching to avoid shutdown hang\n";
                    thread_.detach();
                    return;
                }
            }
            else
            {
                lock.unlock();
            }
            thread_.join();
        }
        catch (...)
        {
            // Destructors must not throw (and shutdown is called from the dtor).
        }
    }

    /**
     * @brief Attempt to launch a background job.
     *
     * Returns `false` if a job is already running (concurrent start rejected).
     * The work function is called on a new thread; its return value is stored
     * as the result. Exceptions are caught and stored as an error string.
     * Work is invoked exactly once per `try_start`; the thread exits when
     * `work` returns or throws.
     *
     * @param work Callable returning `TResult`. Invoked on the background thread.
     * @return `true` if the job was started, `false` if one is already running.
     */
    bool try_start(std::function<TResult()> work)
    {
        std::unique_lock<std::mutex> lock(state_->mutex);

        // Check-and-set under the same mutex that the thread lambda uses to
        // clear running_.  This closes the race window where a previous
        // thread's store(false) + notify could interleave with exchange(true).
        if (state_->running.load())
            return false;
        state_->running.store(true);

        state_->result.reset();
        state_->last_error.clear();
        state_->cancel_requested.store(false);

        if (thread_.joinable())
        {
            // Previous thread already completed (running was false to reach here).
            // Unlock before join to avoid deadlock with the lambda.
            lock.unlock();
            thread_.join();
            lock.lock();
        }

        try
        {
            // Capture state_ by value so the worker keeps the State alive
            // even if `*this` is destroyed before the work completes.
            thread_ = std::thread(
                [state = state_, work = std::move(work)]()
                {
                    try
                    {
                        auto result = work();
                        std::lock_guard<std::mutex> lk(state->mutex);
                        state->result = std::move(result);
                        state->last_error.clear();
                        state->running.store(false);
                        state->cv.notify_all();
                    }
                    catch (const std::exception& ex)
                    {
                        std::lock_guard<std::mutex> lk(state->mutex);
                        state->last_error = ex.what();
                        state->running.store(false);
                        state->cv.notify_all();
                    }
                    catch (...)
                    {
                        std::lock_guard<std::mutex> lk(state->mutex);
                        state->last_error = "Unknown error";
                        state->running.store(false);
                        state->cv.notify_all();
                    }
                });
        }
        catch (...)
        {
            state_->running.store(false);
            throw;
        }

        return true;
    }

    /** Check if the job is currently running (lock-free). */
    [[nodiscard]] bool is_running() const { return state_->running.load(); }

    /**
     * @brief Request cooperative cancellation of the running job.
     *
     * Sets the cancellation flag that work functions can poll via
     * is_cancel_requested() or cancel_token(). This is a non-blocking,
     * lock-free operation. The work function must check the flag
     * periodically and exit voluntarily.
     */
    void request_cancel() { state_->cancel_requested.store(true); }

    /**
     * @brief Check if cancellation has been requested (lock-free).
     * @return `true` if request_cancel() has been called since the
     *         last try_start().
     */
    [[nodiscard]] bool is_cancel_requested() const { return state_->cancel_requested.load(); }

    /**
     * @brief Return a reference to the cancellation token for passing to work functions.
     *
     * The returned reference is valid for the lifetime of the underlying
     * shared State, which outlives `*this` if a worker is still running.
     * Callers should not retain the reference past the BackgroundJob's
     * lifetime; in practice only the worker itself uses this token.
     */
    [[nodiscard]] const std::atomic<bool>& cancel_token() const { return state_->cancel_requested; }

    /**
     * @brief Read the job result under the mutex.
     *
     * The callback receives `(has_result, result, error)` while the mutex
     * is held, so the caller can safely inspect all fields. Returns
     * whatever the callback returns.
     *
     * @param fn Callable with signature `auto(bool has_result, const TResult* result, const
     * std::string& error)`. The pointer is null when `has_result` is false.
     * @note The callback runs while the internal mutex is held; it must
     *       not call back into this BackgroundJob (e.g. `try_start`,
     *       `request_cancel`) - that would deadlock.
     */
    template <typename Fn>
    auto read_result(Fn&& fn) const
    {
        std::lock_guard<std::mutex> lock(state_->mutex);
        const TResult* ptr = state_->result.has_value() ? &*state_->result : nullptr;
        return fn(state_->result.has_value(), ptr, state_->last_error);
    }

private:
    std::shared_ptr<State> state_;
    std::thread thread_;
};

}  // namespace mo2server

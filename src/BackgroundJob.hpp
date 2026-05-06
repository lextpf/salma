#pragma once

#include "Logger.hpp"

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
 * Wraps the recurring pattern of an atomic running flag, a mutex-guarded
 * result, and a joinable background thread. Callers supply a work function
 * returning a `TResult`; the template handles start gating, thread join,
 * exception capture and result access.
 *
 * Header-only, and declared in namespace `mo2server` although it knows nothing
 * about Crow or HTTP. It is grouped with the shared support types for that
 * reason, even though the REST controllers are its only consumers.
 *
 * @tparam TResult Result type stored on completion. Must be move-constructible
 *         (a `static_assert` in the class body rejects anything else) and a
 *         complete object type, because the result is held in a
 *         `std::optional<TResult>`.
 *
 * ## Cooperative Cancellation
 *
 * Work functions poll `cancel_token()` or `is_cancel_requested()`. The
 * destructor sets the token and waits up to the grace period. A worker that
 * exits inside the grace period is joined and nothing leaks; the thread is
 * detached only when the grace period expires with the worker still running, so
 * a non-cooperative worker cannot block process shutdown forever.
 *
 * Detach is safe because every piece of state the worker touches lives in a
 * heap-allocated `State` held by a `shared_ptr` that the worker captures by
 * value, so the state outlives `*this`. What detach does leak is the thread
 * itself and whatever the work holds, until the work returns. Long-running
 * loops must poll the token.
 *
 * ## Lifecycle
 *
 * `State.result` and `State.last_error` below are members of the private
 * `State` struct, reached as `state_->result` and `state_->last_error`. There
 * are no `result_` or `last_error_` members.
 *
 * `cancelling` is conceptual, not stored. `request_cancel()` sets
 * `state_->cancel_requested` while `state_->running` stays true, so the job
 * reports itself running until the worker observes the flag and returns.
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
 *     running --> idle: work() returns, State.result set
 *     running --> idle: work() throws, State.last_error set
 *     running --> cancelling: request_cancel() - still running, flag set
 *     cancelling --> idle: work() observes the token and exits
 * ```
 *
 * ## Shutdown timing
 *
 * The destructor, and an explicit `shutdown()`, bound the wait at the grace
 * period:
 *
 * $$t_{shutdown} \le \texttt{kShutdownGrace} = 10\,\text{s}$$
 *
 * Within the grace the thread is joined. Past it the thread is detached and the
 * worker keeps running against the `State` it captured by `shared_ptr`.
 */
template <typename TResult>
class BackgroundJob
{
    static_assert(std::is_move_constructible_v<TResult>,
                  "BackgroundJob requires TResult to be move-constructible");

    // Every piece of state the worker touches. Held by shared_ptr and captured
    // by value in the worker, so it survives the owning BackgroundJob. That is
    // what makes detach-on-shutdown safe.
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
     * observe it and exit, then joins if it exited and detaches if it did not.
     * Detaching is safe because the worker captures `state_` by value, so the
     * State outlives `*this`.
     *
     * Idempotent across sequential calls: later invocations see
     * `thread_.joinable() == false` and return at once. The destructor calls
     * this automatically.
     *
     * @warning Not safe to call concurrently on the same BackgroundJob.
     *          `state_->mutex` protects the State, but `thread_` is unguarded,
     *          so concurrent shutdowns race on `thread_.join()` and
     *          `thread_.detach()`. Owners that need parallel shutdown must
     *          serialize externally.
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
                    // The worker missed the cancellation flag. Detach and warn.
                    // The State stays alive while the worker holds its
                    // shared_ptr; what leaks is the std::thread handle and
                    // whatever the work captured.
                    //
                    // Route through Logger so a host that registered a callback
                    // sees the warning. Fall back to stderr if Logger throws:
                    // shutdown() runs from the destructor and must not
                    // propagate.
                    try
                    {
                        mo2core::Logger::instance().log_warning(
                            "[BackgroundJob] Worker did not stop within grace period; "
                            "detaching to avoid shutdown hang");
                    }
                    catch (...)
                    {
                        std::cerr << "[BackgroundJob] Worker did not stop within grace period; "
                                     "detaching to avoid shutdown hang\n";
                    }
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
     * @p work runs once on a new thread; its return value becomes the result,
     * and an exception from it becomes the stored error string. The thread
     * exits when @p work returns or throws.
     *
     * A successful start first clears the previous result, error and
     * cancellation flag, so the stored state always belongs to the newest run.
     *
     * **Blocking:** when a previous run has finished but its thread has not
     * been reaped, this joins that thread before starting the new one. The wait
     * is short, because the worker has already returned, but it is not zero.
     *
     * @param work Callable returning `TResult`. Invoked on the background thread.
     * @return `true` if the job was started, `false` if one is already running.
     * @throw std::system_error when the OS cannot create the thread. The
     *        running flag is reset to false before the exception propagates, so
     *        a later try_start() can still succeed. Callers that treat this as
     *        a plain bool need a catch.
     */
    bool try_start(std::function<TResult()> work)
    {
        std::unique_lock<std::mutex> lock(state_->mutex);

        // Check-and-set under the same mutex the worker lambda uses to clear
        // state_->running. That closes the window where a previous worker's
        // store(false) plus notify could interleave with an exchange(true).
        if (state_->running.load())
            return false;
        state_->running.store(true);

        state_->result.reset();
        state_->last_error.clear();
        state_->cancel_requested.store(false);

        if (thread_.joinable())
        {
            // The previous worker has already returned; running was false to
            // reach here. Unlock before join, or the lambda deadlocks on it.
            lock.unlock();
            thread_.join();
            lock.lock();
        }

        try
        {
            // Capture state_ by value so the State survives if `*this` is
            // destroyed before the work completes.
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

    /// Check if the job is currently running (lock-free, never blocks).
    [[nodiscard]] bool is_running() const { return state_->running.load(); }

    /**
     * @brief Request cooperative cancellation of the running job.
     *
     * Sets the flag that work functions poll via is_cancel_requested() or
     * cancel_token(). Lock-free and non-blocking. Cancellation only takes
     * effect if the work function checks the flag and returns.
     */
    void request_cancel() { state_->cancel_requested.store(true); }

    /**
     * @brief Check if cancellation has been requested (lock-free).
     * @return `true` if request_cancel() has been called since the
     *         last try_start().
     */
    [[nodiscard]] bool is_cancel_requested() const { return state_->cancel_requested.load(); }

    /**
     * @brief Return the cancellation token, for passing into a work function.
     *
     * The reference stays valid for the lifetime of the shared State, which
     * outlives `*this` when a worker is still running. Do not retain it beyond
     * the BackgroundJob; in practice only the worker reads this token.
     */
    [[nodiscard]] const std::atomic<bool>& cancel_token() const { return state_->cancel_requested; }

    /**
     * @brief Read the job result under the mutex.
     *
     * @p fn sees a consistent `(has_result, result, error)` triple because the
     * mutex is held across the call. Returns whatever @p fn returns.
     *
     * @param fn Callable with signature `auto(bool has_result, const TResult* result, const
     * std::string& error)`. The pointer is null when `has_result` is false.
     * @note @p fn runs with the internal mutex held, so it must not call
     *       `try_start`, `shutdown` or another `read_result` on this
     *       BackgroundJob: those take the same mutex and deadlock. The
     *       atomic-only operations (`request_cancel`, `is_cancel_requested`,
     *       `is_running`, `cancel_token`) are safe from inside it.
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

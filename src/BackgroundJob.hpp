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
 * @brief Runs one asynchronous job and stores its result.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup Core
 *
 * Cancellation is cooperative. Destruction waits 10 seconds, then detaches a
 * worker that has not stopped. Shared job state survives detachment. Captured
 * references do not gain ownership; their targets must outlive the worker.
 *
 * ### :material-state-machine: Lifecycle
 *
 * ```mermaid
 * stateDiagram-v2
 *     [*] --> idle
 *     idle --> running: try_start
 *     running --> idle: work returns
 *     running --> stopping: shutdown
 *     stopping --> idle: worker stops in grace
 *     stopping --> detached: grace expires
 * ```
 *
 * @tparam TResult Move-constructible result type.
 */
template <typename TResult>
class BackgroundJob
{
    static_assert(std::is_move_constructible_v<TResult>,
                  "BackgroundJob requires TResult to be move-constructible");

    // The worker captures this state so detach cannot leave references to the owner.
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
    /// Shutdown grace period before detach.
    static constexpr std::chrono::seconds kShutdownGrace{10};

    /**
     * @fn BackgroundJob::BackgroundJob()
     * @brief Initialize an idle job with independent shared state.
     * @author Alex (<https://github.com/lextpf>)
     */
    BackgroundJob()
        : state_(std::make_shared<State>())
    {
    }

    /**
     * @fn BackgroundJob::~BackgroundJob()
     * @brief Request cancellation and wait for the shutdown grace period.
     * @author Alex (<https://github.com/lextpf>)
     */
    ~BackgroundJob() { shutdown(kShutdownGrace); }

    BackgroundJob(const BackgroundJob&) = delete;
    BackgroundJob& operator=(const BackgroundJob&) = delete;

    /**
     * @fn void BackgroundJob::shutdown(std::chrono::milliseconds grace)
     * @brief Limits shutdown blocking to the supplied grace period.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The call joins a cooperative worker and detaches after the grace period.
     * Sequential calls are idempotent. All exceptions are contained because the
     * destructor uses this operation.
     *
     * @param grace Maximum wait, in milliseconds.
     * @warning Do not call this concurrently on the same object. `thread_` is unguarded.
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
                    // The shared state keeps a detached worker valid until it returns.
                    // Use Logger for registered callbacks. Standard error remains available during teardown.
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
            // Shutdown is also the destructor path and must not throw.
        }
    }

    /**
     * @fn bool BackgroundJob::try_start(std::function<TResult()> work)
     * @brief Rejects overlap and resets prior state before dispatch.
     * @author Alex (<https://github.com/lextpf>)
     *
     * A new run clears the prior result, error, and cancellation state. A rejected
     * start leaves them unchanged. Do not race start calls with shutdown.
     *
     * ### :material-alert-circle-outline: Failures
     *
     * Standard exceptions become error text. Thread creation failures propagate
     * after the running flag is cleared.
     *
     * @param work Callable invoked once on the worker thread.
     * @return `true` after start, or `false` when another run is active.
     */
    bool try_start(std::function<TResult()> work)
    {
        std::unique_lock<std::mutex> lock(state_->mutex);

        // Check and set under the mutex used by the worker to clear the flag.
        if (state_->running.load())
            return false;
        state_->running.store(true);

        state_->result.reset();
        state_->last_error.clear();
        state_->cancel_requested.store(false);

        if (thread_.joinable())
        {
            // Unlock before joining because the completed worker also takes this mutex.
            lock.unlock();
            thread_.join();
            lock.lock();
        }

        try
        {
            // Capture shared state so detach cannot outlive worker data.
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

    /**
     * @fn bool BackgroundJob::is_running() const
     * @brief Report whether a worker is still marked running.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return An advisory snapshot; the worker can finish immediately after the read.
     */
    [[nodiscard]] bool is_running() const { return state_->running.load(); }

    /**
     * @fn void BackgroundJob::request_cancel()
     * @brief Request cooperative cancellation without waiting.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The worker must poll the token. The next accepted start clears the request.
     */
    void request_cancel() { state_->cancel_requested.store(true); }

    /**
     * @fn bool BackgroundJob::is_cancel_requested() const
     * @brief Inspect the cancellation request for the current run.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return `true` after cancellation is requested, including during shutdown.
     */
    [[nodiscard]] bool is_cancel_requested() const { return state_->cancel_requested.load(); }

    /**
     * @fn const std::atomic<bool>& BackgroundJob::cancel_token() const
     * @brief Expose the cancellation flag for cooperative workers.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The reference does not extend ownership. A detached worker keeps it valid
     * only until that worker exits.
     *
     * @return The flag borrowed from the shared job state.
     */
    [[nodiscard]] const std::atomic<bool>& cancel_token() const { return state_->cancel_requested; }

    /**
     * @fn template <typename Fn> auto BackgroundJob::read_result(Fn&& fn) const
     * @brief Reads the result and error as one synchronized snapshot.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The callback runs while the internal mutex is held. Its result pointer is
     * null until work succeeds. Do not retain the pointer or the error reference
     * after the callback; a new run can clear both.
     *
     * @tparam Fn Callback type.
     * @param fn Callable that accepts the presence flag, result pointer, and error text.
     * @return The callback result.
     * @warning The callback must not call an operation that locks this job.
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

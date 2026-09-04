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
 * @brief runs one asynchronous job and stores its result.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Core
 *
 * work must poll the cancellation token. destruction waits 10 seconds, then
 * detaches a non-cooperative worker. shared state remains valid after detach, but
 * the worker and its captured resources remain live until work returns.
 *
 * ### :material-state-machine: lifecycle
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
 * @tparam TResult move-constructible result type.
 */
template <typename TResult>
class BackgroundJob
{
    static_assert(std::is_move_constructible_v<TResult>,
                  "BackgroundJob requires TResult to be move-constructible");

    // the worker captures this state so detach cannot leave references to the owner.
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
    /// shutdown grace period before detach.
    static constexpr std::chrono::seconds kShutdownGrace{10};

    BackgroundJob()
        : state_(std::make_shared<State>())
    {
    }

    ~BackgroundJob() { shutdown(kShutdownGrace); }

    BackgroundJob(const BackgroundJob&) = delete;
    BackgroundJob& operator=(const BackgroundJob&) = delete;

    /**
     * @fn void BackgroundJob::shutdown(std::chrono::milliseconds grace)
     * @brief limits shutdown blocking to the supplied grace period.
     * @author Alex (https://github.com/lextpf)
     *
     * the call joins a cooperative worker and detaches after the grace period.
     * sequential calls are idempotent. all exceptions are contained because the
     * destructor uses this operation.
     *
     * @param grace maximum wait, in milliseconds.
     * @warning do not call this concurrently on the same object. `thread_` is unguarded.
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
                    // the shared state keeps a detached worker valid until it returns.
                    // use Logger for registered callbacks. stderr remains safe during teardown.
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
            // shutdown is also the destructor path and must not throw.
        }
    }

    /**
     * @fn bool BackgroundJob::try_start(std::function<TResult()> work)
     * @brief rejects overlap and resets prior state before dispatch.
     * @author Alex (https://github.com/lextpf)
     *
     * a new run clears the prior result, error, and cancellation state.
     *
     * ### :material-alert-circle-outline: failures
     *
     * standard exceptions become error text. thread creation failures propagate
     * after the running flag is cleared.
     *
     * @param work callable invoked once on the worker thread.
     * @return `true` after start, or `false` when another run is active.
     */
    bool try_start(std::function<TResult()> work)
    {
        std::unique_lock<std::mutex> lock(state_->mutex);

        // check and set under the mutex used by the worker to clear the flag.
        if (state_->running.load())
            return false;
        state_->running.store(true);

        state_->result.reset();
        state_->last_error.clear();
        state_->cancel_requested.store(false);

        if (thread_.joinable())
        {
            // unlock before joining because the completed worker also takes this mutex.
            lock.unlock();
            thread_.join();
            lock.lock();
        }

        try
        {
            // capture shared state so detach cannot outlive worker data.
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

    [[nodiscard]] bool is_running() const { return state_->running.load(); }

    void request_cancel() { state_->cancel_requested.store(true); }

    [[nodiscard]] bool is_cancel_requested() const { return state_->cancel_requested.load(); }

    /**
     * @fn const std::atomic<bool>& BackgroundJob::cancel_token() const
     * @brief shares cancellation state with work that outlives its owner.
     * @author Alex (https://github.com/lextpf)
     *
     * @return a reference that remains valid while detached work holds shared state.
     */
    [[nodiscard]] const std::atomic<bool>& cancel_token() const { return state_->cancel_requested; }

    /**
     * @fn template <typename Fn> auto BackgroundJob::read_result(Fn&& fn) const
     * @brief reads the result and error as one synchronized snapshot.
     * @author Alex (https://github.com/lextpf)
     *
     * the callback runs while the internal mutex is held.
     *
     * @tparam Fn callback type.
     * @param fn callable that accepts the presence flag, result pointer, and error text.
     * @return the callback result.
     * @warning the callback must not call an operation that locks this job.
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

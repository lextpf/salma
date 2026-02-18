#pragma once

#include <crow.h>

namespace mo2server
{

/**
 * @class SecurityMiddleware
 * @brief Crow middleware that enforces Origin allowlist + CSRF token policy.
 * @author Alex (https://github.com/lextpf)
 *
 * Replaces Crow's `CORSHandler` so the dashboard can mint short-lived
 * CSRF tokens and require them on every state-changing request. The
 * middleware:
 *
 *   - Handles `OPTIONS` preflight requests by echoing the request's
 *     `Origin` header (when allowlisted) along with the allowed methods
 *     and headers, then short-circuits with 204. Non-allowlisted origins
 *     receive 403 with no CORS headers, which causes the browser to
 *     refuse the subsequent actual request.
 *   - For state-changing methods (POST, PUT, DELETE, PATCH): rejects
 *     with 403 if the `X-Salma-Csrf` header is missing or does not match
 *     the per-process token in @ref mo2core::SecurityContext. Also
 *     rejects requests whose `Origin` header is set but not in the
 *     allowlist.
 *   - For safe methods (GET, HEAD): no token check.
 *   - On every response with an allowlisted `Origin`: adds
 *     `Access-Control-Allow-Origin: <echoed origin>` and `Vary: Origin`
 *     in @ref after_handle.
 *
 * The middleware reads from @ref mo2core::SecurityContext for the token
 * and allowlist - both are fixed for the process lifetime, so no
 * synchronization is needed in the hot path.
 *
 * @see mo2core::SecurityContext
 */
struct SecurityMiddleware
{
    struct context
    {
    };

    /**
     * @brief Run before the route handler.
     *
     * Short-circuits preflight, missing-token, and bad-origin requests
     * by setting @p res and calling `res.end()`. Allowed requests fall
     * through to the route handler unmodified.
     */
    void before_handle(crow::request& req, crow::response& res, context& ctx);

    /**
     * @brief Run after the route handler.
     *
     * Adds `Access-Control-Allow-Origin` and `Vary: Origin` headers when
     * the request's `Origin` is allowlisted. No-op otherwise.
     */
    void after_handle(crow::request& req, crow::response& res, context& ctx);
};

}  // namespace mo2server

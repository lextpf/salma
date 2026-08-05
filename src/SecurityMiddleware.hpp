#pragma once

#include <crow.h>

namespace mo2server
{

/**
 * @struct SecurityMiddleware
 * @brief Enforces browser origin and CSRF policy for Crow routes.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup SecurityMiddleware
 *
 * ### :material-shield-lock: Request policy
 *
 * | Request                     | Rule                                      |
 * |-----------------------------|-------------------------------------------|
 * | `OPTIONS`                   | Require an allowlisted `Origin`; send 204 |
 * | POST, PUT, DELETE, or PATCH | Check `Origin` when present, then CSRF    |
 * | All other methods           | Continue without either check             |
 *
 * Rejected mutations return HTTP 403. The exact CSRF error text is consumed by
 * the dashboard retry path. A missing `Origin` is allowed because same-origin
 * clients can omit it; the CSRF token remains required.
 *
 * ### :material-transit-connection-variant: Request flow
 *
 * ```mermaid
 * flowchart TD
 *     request --> options{OPTIONS?}
 *     options -- yes --> allowed{origin allowed?}
 *     allowed -- yes --> preflight[204]
 *     allowed -- no --> reject[403]
 *     options -- no --> mutation{state changing?}
 *     mutation -- no --> route[route handler]
 *     mutation -- yes --> origin{present origin rejected?}
 *     origin -- yes --> reject
 *     origin -- no --> token{valid CSRF token?}
 *     token -- yes --> route
 *     token -- no --> reject
 * ```
 *
 * ### :material-link-variant: CORS response headers
 *
 * `after_handle` adds CORS response headers only for allowlisted origins.
 *
 * @warning Preflight appends `Access-Control-Allow-Origin` and `Vary` in both
 *          hooks. These duplicate headers can prevent browser CORS acceptance.
 *
 * @see mo2core::SecurityContext
 */
struct SecurityMiddleware
{
    struct context
    {
    };

    /**
     * @fn void SecurityMiddleware::before_handle(crow::request&, crow::response&, context&)
     * @brief Short-circuits preflight and rejected mutations before routing.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Rejected requests end the response and skip the route handler. Token
     * comparison uses `mo2core::constant_time_equals`.
     *
     * @param req Request to inspect.
     * @param res Response completed on rejection or preflight.
     * @param ctx Unused middleware context.
     */
    void before_handle(crow::request& req, crow::response& res, context& ctx);

    /**
     * @fn void SecurityMiddleware::after_handle(crow::request&, crow::response&, context&)
     * @brief Withholds CORS response headers from rejected origins.
     * @author Alex (<https://github.com/lextpf>)
     *
     * Crow also calls this after a `before_handle` short circuit.
     *
     * @param req Request that supplied the origin.
     * @param res Response to update.
     * @param ctx Unused middleware context.
     */
    void after_handle(crow::request& req, crow::response& res, context& ctx);
};

}  // namespace mo2server

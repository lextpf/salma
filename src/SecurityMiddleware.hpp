#pragma once

#include <crow.h>

namespace mo2server
{

/**
 * @class SecurityMiddleware
 * @brief enforces browser origin and CSRF policy for Crow routes.
 * @author Alex (https://github.com/lextpf)
 * @ingroup SecurityMiddleware
 *
 * ### :material-shield-lock: request policy
 *
 * | request                         | rule                                      |
 * |---------------------------------|-------------------------------------------|
 * | `OPTIONS`                       | require an allowlisted `Origin`; send 204 |
 * | POST, PUT, DELETE, or PATCH     | check `Origin` when present, then CSRF    |
 * | all other methods               | continue without either check             |
 *
 * rejected mutations return HTTP 403. the exact CSRF error text is consumed by
 * the dashboard retry path. a missing `Origin` is allowed because same-origin
 * clients can omit it; the CSRF token remains required.
 *
 * ### :material-transit-connection-variant: request flow
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
 * @warning preflight currently appends `Access-Control-Allow-Origin` and `Vary`
 *          twice. fix this before serving the dashboard across origins.
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
     * @brief short-circuits preflight and rejected mutations before routing.
     * @author Alex (https://github.com/lextpf)
     *
     * rejected requests end the response and skip the route handler. token
     * comparison uses `mo2core::constant_time_equals`.
     *
     * @param req request to inspect.
     * @param res response completed on rejection or preflight.
     * @param ctx unused middleware context.
     */
    void before_handle(crow::request& req, crow::response& res, context& ctx);

    /**
     * @fn void SecurityMiddleware::after_handle(crow::request&, crow::response&, context&)
     * @brief withholds CORS response headers from rejected origins.
     * @author Alex (https://github.com/lextpf)
     *
     * Crow also calls this after a `before_handle` short circuit.
     *
     * @param req request that supplied the origin.
     * @param res response to update.
     * @param ctx unused middleware context.
     */
    void after_handle(crow::request& req, crow::response& res, context& ctx);
};

}  // namespace mo2server

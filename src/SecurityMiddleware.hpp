#pragma once

#include <crow.h>

namespace mo2server
{

/**
 * @class SecurityMiddleware
 * @brief Crow middleware that enforces the Origin allowlist and CSRF token policy.
 * @author Alex (https://github.com/lextpf)
 * @ingroup SecurityMiddleware
 *
 * Replaces Crow's `CORSHandler` so the dashboard can require a CSRF
 * token on every state-changing request. This middleware validates the
 * token and never creates one. `mo2core::SecurityContext` generates a
 * single 64-hex-character token when its singleton is first touched, and
 * `GET /api/csrf-token` (registered in `main.cpp`) hands that same value
 * to the dashboard. The token is fixed for the life of the process:
 * nothing rotates it and nothing expires it, so restarting the server is
 * the only way to change it. `mo2core::SecurityContext` is the
 * authoritative description of the token and the allowlist, including
 * the entropy limit of the generator; this page describes only how they
 * are applied to a request.
 *
 * The token endpoint is a GET, so this middleware does not gate it. CORS
 * is its protection instead: a cross-origin page can issue the request
 * but cannot read the response body, because `after_handle` adds
 * `Access-Control-Allow-Origin` only for allowlisted origins.
 *
 * ## :material-shield-check: Request flow
 *
 * ```mermaid
 * ---
 * config:
 *   theme: dark
 *   look: handDrawn
 * ---
 * flowchart TD
 *     A["request"] --> B{"method is OPTIONS?"}
 *     B -- yes --> C{"Origin present and allowlisted?"}
 *     C -- yes --> D["204 + CORS and preflight headers"]
 *     C -- no --> E["403, empty body"]
 *     B -- no --> F{"state-changing? POST PUT DELETE PATCH"}
 *     F -- no --> G["route handler"]
 *     F -- yes --> H{"Origin present but not allowlisted?"}
 *     H -- yes --> I["403 'origin not allowed'"]
 *     H -- no --> J{"X-Salma-Csrf matches the process token?"}
 *     J -- no --> K["403 'csrf token missing or invalid'"]
 *     J -- yes --> G
 *     D -.-> L["after_handle adds ACAO + Vary when Origin is allowlisted"]
 *     E -.-> L
 *     I -.-> L
 *     K -.-> L
 *     G --> L
 *     L --> M["response"]
 * ```
 *
 * The dotted edges are the short-circuit paths. They skip the route
 * handler but not `after_handle`: Crow's `middleware_call_helper` calls
 * `after_handle` inline as soon as `before_handle` returns with the
 * response completed, and the connection-level after-handler flag only
 * suppresses a second call from `complete_request`. `after_handle`
 * therefore runs exactly once on every path above. Two consequences:
 *
 * - A 403 answering a request whose `Origin` is allowlisted still
 *   carries `Access-Control-Allow-Origin` and `Vary`, so a browser page
 *   on that origin can read the JSON error body. A rejection whose
 *   `Origin` is absent or not allowlisted carries neither, and the
 *   browser hides the body from the calling page. The dashboard does not
 *   rely on either case: production serves it from `mo2-server` itself
 *   and the Vite dev server proxies `/api`, so its own requests are
 *   same-origin both ways.
 * - The 204 preflight leaves with `Access-Control-Allow-Origin` and
 *   `Vary` twice, because `before_handle` and `after_handle` both apply
 *   them with `crow::response::add_header`, which appends to a multimap
 *   instead of replacing. A browser joins the repeats into one
 *   comma-separated value that matches neither the request's origin nor
 *   `*`, so the CORS check fails. Move `apply_cors_headers` to
 *   `set_header` before serving a genuinely cross-origin dashboard.
 *
 * ## :material-api: Preflight response
 *
 * An `OPTIONS` request with an allowlisted `Origin` is answered with 204
 * and this header set, with the first two repeated for the reason given
 * above. A frontend that adds a custom request header or a new HTTP verb
 * has to extend the set, or the browser blocks the real request.
 *
 * | Header | Value |
 * |--------|-------|
 * | `Access-Control-Allow-Origin` | the request's `Origin`, echoed back |
 * | `Vary` | `Origin` |
 * | `Access-Control-Allow-Methods` | `GET, POST, PUT, DELETE, OPTIONS` |
 * | `Access-Control-Allow-Headers` | `Content-Type, X-Salma-Csrf` |
 * | `Access-Control-Max-Age` | `600` (seconds) |
 *
 * An `OPTIONS` request whose `Origin` is missing or not allowlisted gets
 * 403 with an empty body, which makes the browser refuse the actual
 * request that would have followed.
 *
 * ## :material-alert-circle-outline: Rejection bodies
 *
 * The two state-changing rejections return 403 with
 * `Content-Type: application/json` and the bodies below, byte for byte.
 * Treat the strings as a contract: `web/src/api.ts` matches on the
 * second one to decide whether to refetch the token and retry once.
 *
 * | Condition | Body |
 * |-----------|------|
 * | `Origin` present and not allowlisted | `{"error":"origin not allowed"}` |
 * | `X-Salma-Csrf` missing or wrong | `{"error":"csrf token missing or invalid"}` |
 *
 * ## :material-information-outline: Deliberate gaps
 *
 * - **A missing `Origin` is not a rejection.** Same-origin browser
 *   fetches and command-line clients legitimately omit the header, while
 *   a cross-site forgery attempt from an attacker page always carries
 *   one. A request with no `Origin` falls through to the CSRF check,
 *   which is the real gate.
 * - **Safe methods are not checked at all.** GET and HEAD pass with no
 *   token check and no origin check. A GET from a disallowed origin is
 *   served, but `after_handle` omits `Access-Control-Allow-Origin`, so
 *   the browser refuses to hand the body to the calling page.
 * - **Any method that is neither OPTIONS nor state-changing is
 *   ungated**, by the same rule.
 *
 * ## :material-help: Thread safety
 *
 * Both handlers read `mo2core::SecurityContext` for the token and the
 * allowlist. Both values are fixed once the singleton is constructed, so
 * the hot path needs no synchronization. The middleware itself holds no
 * state.
 *
 * @see mo2core::SecurityContext
 */
struct SecurityMiddleware
{
    /**
     * @brief Per-request middleware state. Intentionally empty.
     *
     * Crow requires the nested type, but nothing needs to be carried
     * from before_handle() to after_handle(): both re-read the `Origin`
     * header from the request.
     */
    struct context
    {
    };

    /**
     * @brief Run before the route handler.
     *
     * Short-circuits preflight, bad-origin and missing-or-wrong-token
     * requests by setting @p res and calling `res.end()`. That skips the
     * route handler only: Crow still calls after_handle() for the
     * response, so the CORS headers are added there as usual. An allowed
     * request falls through to the route handler with @p res untouched.
     *
     * The token comparison uses `mo2core::constant_time_equals` rather
     * than `std::string::operator==`, so the runtime does not leak how
     * many leading characters of the token matched.
     *
     * @param req Incoming request. Read only; nothing is modified.
     * @param res Response to fill in on rejection. Left untouched when
     *        the request is allowed through.
     * @param ctx Unused. The context type carries no state.
     */
    void before_handle(crow::request& req, crow::response& res, context& ctx);

    /**
     * @brief Run after the route handler, or after a before_handle()
     *        short-circuit.
     *
     * Adds `Access-Control-Allow-Origin` (the request's own origin,
     * echoed back) and `Vary: Origin` when the request's `Origin` is
     * present and allowlisted. No-op otherwise.
     *
     * Crow calls this for every request that reaches this middleware,
     * responses before_handle() ended with `res.end()` included, and
     * calls it once: the inline call from `middleware_call_helper` and
     * the connection-level call from `complete_request` are mutually
     * exclusive. Both headers go on with `crow::response::add_header`,
     * which appends rather than replaces, so on the preflight path,
     * where before_handle() has already added the same two, each ends up
     * in the response twice.
     *
     * @param req Incoming request. Read only.
     * @param res Response to add headers to.
     * @param ctx Unused. The context type carries no state.
     */
    void after_handle(crow::request& req, crow::response& res, context& ctx);
};

}  // namespace mo2server

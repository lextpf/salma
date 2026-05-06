// SecurityMiddleware - the Origin allowlist and the CSRF gate, applied to every
// request before any route handler runs.
//
// before_handle decides in this order, and the order is the policy:
//
//   OPTIONS ?                       yes -> Origin allowed ? 204 + preflight
//     |                                                     headers : 403, end
//     no
//     |
//   state-changing method ?         no  -> pass through, no checks
//     (POST/PUT/DELETE/PATCH)
//     |
//     yes
//     |
//   Origin present but not allowed ? yes -> 403 "origin not allowed"
//     |
//     no  (allowed, or absent)
//     |
//   X-Salma-Csrf present and equal  no  -> 403 "csrf token missing or invalid"
//   to the process token ?
//     |
//     yes -> pass through to the handler
//
// GET and HEAD are never gated, which is what lets the dashboard fetch the token
// from /api/csrf-token in the first place.
//
// after_handle adds the CORS response headers, and only for an allowed Origin.
// Crow runs it even on a response before_handle already completed, so a 403 on
// a request that carried an allowed Origin still gets the headers the browser
// needs to expose the body. That is what lets web/src/api.ts read the error
// text and retry once when the token is stale.
//
// The token itself lives in mo2core::SecurityContext, in salma-support, so the
// policy helpers can be unit-tested without linking Crow
// (tests/security_context_test.cpp).

#include "SecurityMiddleware.hpp"
#include "SecurityContext.hpp"

#include <nlohmann/json.hpp>

namespace mo2server
{

namespace
{

constexpr const char* kAllowedMethods = "GET, POST, PUT, DELETE, OPTIONS";
constexpr const char* kAllowedHeaders = "Content-Type, X-Salma-Csrf";
constexpr const char* kPreflightMaxAge = "600";

void apply_cors_headers(crow::response& res, const std::string& origin)
{
    res.add_header("Access-Control-Allow-Origin", origin);
    res.add_header("Vary", "Origin");
}

void apply_preflight_headers(crow::response& res, const std::string& origin)
{
    apply_cors_headers(res, origin);
    res.add_header("Access-Control-Allow-Methods", kAllowedMethods);
    res.add_header("Access-Control-Allow-Headers", kAllowedHeaders);
    res.add_header("Access-Control-Max-Age", kPreflightMaxAge);
}

void send_403(crow::response& res, const char* error_message)
{
    nlohmann::json body = {{"error", error_message}};
    res.code = 403;
    res.set_header("Content-Type", "application/json");
    res.body = body.dump();
    res.end();
}

}  // namespace

void SecurityMiddleware::before_handle(crow::request& req, crow::response& res, context&)
{
    // Preflight and CSRF enforcement are split deliberately. A preflight is
    // judged on Origin alone, because a browser never attaches X-Salma-Csrf to
    // an OPTIONS probe; a state-changing request goes through the token check
    // as well.
    //
    // The absent-Origin path is the subtle one. Same-origin fetches and
    // curl-style clients legitimately omit Origin, while a cross-site forgery
    // attempt from an attacker page always carries one the browser set. Absent
    // therefore means "not cross-origin" and falls through to the token check,
    // which is the real gate: the token is fetched by a separate same-origin
    // GET that an attacker page cannot read. A present but disallowed Origin is
    // rejected before the token is looked at.
    const std::string& origin = req.get_header_value("Origin");
    const auto& sec = mo2core::SecurityContext::instance();
    const bool origin_allowed = !origin.empty() && sec.is_origin_allowed(origin);

    if (req.method == crow::HTTPMethod::OPTIONS)
    {
        if (origin_allowed)
        {
            apply_preflight_headers(res, origin);
            res.code = 204;
        }
        else
        {
            res.code = 403;
        }
        res.end();
        return;
    }

    const std::string method = std::string(crow::method_name(req.method));
    if (!mo2core::is_state_changing(method))
    {
        return;
    }

    if (!origin.empty() && !origin_allowed)
    {
        send_403(res, "origin not allowed");
        return;
    }

    // constant_time_equals rather than operator==, so the comparison cannot leak
    // token bytes through early-exit timing.
    const std::string& token = req.get_header_value("X-Salma-Csrf");
    if (token.empty() || !mo2core::constant_time_equals(token, sec.csrf_token()))
    {
        send_403(res, "csrf token missing or invalid");
        return;
    }
}

void SecurityMiddleware::after_handle(crow::request& req, crow::response& res, context&)
{
    const std::string& origin = req.get_header_value("Origin");
    if (origin.empty())
    {
        return;
    }
    if (!mo2core::SecurityContext::instance().is_origin_allowed(origin))
    {
        return;
    }
    apply_cors_headers(res, origin);
}

}  // namespace mo2server

#include "SecurityMiddleware.h"
#include "SecurityContext.h"

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

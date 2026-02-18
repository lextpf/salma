#include "SecurityContext.h"
#include "Utils.h"

#include <cctype>
#include <cstdlib>

namespace mo2core
{

std::vector<std::string> default_allowed_origins()
{
    return {
        "http://localhost:5000",
        "http://127.0.0.1:5000",
        "http://localhost:3000",
        "http://127.0.0.1:3000",
    };
}

namespace
{

std::string trim(std::string_view s)
{
    auto is_ws = [](unsigned char c) { return std::isspace(c) != 0; };
    size_t begin = 0;
    while (begin < s.size() && is_ws(static_cast<unsigned char>(s[begin])))
    {
        ++begin;
    }
    size_t end = s.size();
    while (end > begin && is_ws(static_cast<unsigned char>(s[end - 1])))
    {
        --end;
    }
    return std::string(s.substr(begin, end - begin));
}

}  // namespace

std::vector<std::string> parse_origin_list(std::string_view csv)
{
    std::vector<std::string> out;
    size_t start = 0;
    while (start <= csv.size())
    {
        size_t comma = csv.find(',', start);
        if (comma == std::string_view::npos)
        {
            comma = csv.size();
        }
        std::string entry = trim(csv.substr(start, comma - start));
        if (!entry.empty())
        {
            out.push_back(std::move(entry));
        }
        if (comma == csv.size())
        {
            break;
        }
        start = comma + 1;
    }
    return out;
}

bool origin_in_allowlist(const std::vector<std::string>& allowlist, std::string_view origin)
{
    if (origin.empty())
    {
        return false;
    }
    for (const auto& allowed : allowlist)
    {
        if (allowed == origin)
        {
            return true;
        }
    }
    return false;
}

bool is_state_changing(std::string_view method)
{
    auto eq = [](std::string_view a, std::string_view b)
    {
        if (a.size() != b.size())
        {
            return false;
        }
        for (size_t i = 0; i < a.size(); ++i)
        {
            if (std::toupper(static_cast<unsigned char>(a[i])) !=
                std::toupper(static_cast<unsigned char>(b[i])))
            {
                return false;
            }
        }
        return true;
    };
    return eq(method, "POST") || eq(method, "PUT") || eq(method, "DELETE") || eq(method, "PATCH");
}

bool constant_time_equals(std::string_view a, std::string_view b)
{
    if (a.size() != b.size())
    {
        return false;
    }
    unsigned int diff = 0;
    for (size_t i = 0; i < a.size(); ++i)
    {
        diff |= static_cast<unsigned char>(a[i]) ^ static_cast<unsigned char>(b[i]);
    }
    return diff == 0;
}

SecurityContext::SecurityContext()
    : csrf_token_(random_hex_string(64))
{
    if (const char* env = std::getenv("SALMA_ALLOWED_ORIGINS"); env && *env)
    {
        allowed_origins_ = parse_origin_list(env);
    }
    if (allowed_origins_.empty())
    {
        allowed_origins_ = default_allowed_origins();
    }
}

SecurityContext& SecurityContext::instance()
{
    static SecurityContext ctx;
    return ctx;
}

bool SecurityContext::is_origin_allowed(std::string_view origin) const
{
    return origin_in_allowlist(allowed_origins_, origin);
}

}  // namespace mo2core

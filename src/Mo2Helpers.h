#pragma once

#include "Logger.h"
#include "Utils.h"

#include <crow.h>
#include <cctype>
#include <charconv>
#include <cstdlib>
#include <filesystem>
#include <format>
#include <nlohmann/json.hpp>
#include <string>

#ifdef _WIN32
#include <windows.h>
#endif

namespace mo2server
{

/// Build a crow::response with JSON body and Content-Type header.
inline crow::response json_response(int code, const nlohmann::json& j)
{
    crow::response res(code, j.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

/// Percent-decode a URL-encoded string (%XX sequences).
inline std::string url_decode(const std::string& src)
{
    std::string out;
    out.reserve(src.size());
    for (size_t i = 0; i < src.size(); ++i)
    {
        if (src[i] == '%' && i + 2 < src.size())
        {
            unsigned int ch = 0;
            auto [ptr, ec] = std::from_chars(src.data() + i + 1, src.data() + i + 3, ch, 16);
            if (ec == std::errc{} && ptr == src.data() + i + 3)
            {
                if (ch == 0)
                {
                    i += 2;
                    continue;
                }
                out += static_cast<char>(ch);
                i += 2;
                continue;
            }
        }
        out += (src[i] == '+') ? ' ' : src[i];
    }
    return out;
}

/// Trim leading and trailing whitespace from a string.
inline std::string trim_copy(const std::string& s)
{
    size_t start = 0;
    while (start < s.size() && std::isspace(static_cast<unsigned char>(s[start])))
    {
        ++start;
    }
    size_t end = s.size();
    while (end > start && std::isspace(static_cast<unsigned char>(s[end - 1])))
    {
        --end;
    }
    return s.substr(start, end - start);
}

#ifdef _WIN32
/// RAII wrapper for Win32 HANDLEs to prevent leaks on early return/exception.
struct HandleGuard
{
    HANDLE h = nullptr;
    HandleGuard() = default;
    ~HandleGuard()
    {
        if (h)
            CloseHandle(h);
    }
    HANDLE release()
    {
        auto tmp = h;
        h = nullptr;
        return tmp;
    }
    HandleGuard(const HandleGuard&) = delete;
    HandleGuard& operator=(const HandleGuard&) = delete;
};
#endif

/// Resolve the MO2 plugin deploy path from config/env.
std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path);

/// Check whether the Salma plugin is installed at the given deploy path.
/// Returns false if an exception occurs (e.g., permissions error).
bool plugin_installed_at(const std::filesystem::path& deploy_path);

}  // namespace mo2server

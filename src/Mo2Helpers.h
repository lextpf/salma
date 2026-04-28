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

/**
 * @brief Build a crow::response with JSON body and Content-Type header.
 * @param code HTTP status code.
 * @param j JSON body to serialize via `dump()`.
 * @return Crow response with `Content-Type: application/json`.
 */
inline crow::response json_response(int code, const nlohmann::json& j)
{
    crow::response res(code, j.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

/**
 * @brief Percent-decode a URL-encoded string (%XX sequences).
 *
 * Also converts `+` to space. Null bytes (`%00`) are silently dropped.
 *
 * @param src URL-encoded input string.
 * @return Decoded string.
 */
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

/**
 * @brief Trim leading and trailing whitespace from a string.
 * @param s Input string.
 * @return A new string with leading/trailing whitespace removed.
 */
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
/** RAII wrapper for Win32 HANDLEs to prevent leaks on early return/exception. */
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

/**
 * @brief Resolve the MO2 plugin deploy path from config/env.
 *
 * Resolution order:
 *
 * 1. `SALMA_DEPLOY_PATH` env var if non-empty - returned verbatim.
 * 2. `mo2_mods_path` parameter if non-empty.
 * 3. `SALMA_MODS_PATH` env var if step 2 was empty.
 *
 * When a mods path is determined (step 2 or 3), the deploy path is
 * derived as `{mods_path.parent_path().parent_path()} / "MO2" / "plugins"`.
 * The two-level walk goes from `<instance>/mods` up to `<instance>` and
 * then into the MO2 plugins directory. Logs an error and returns
 * an empty path if no source is available.
 *
 * @param mo2_mods_path Configured MO2 mods directory (may be empty).
 * @return Resolved deploy path, or empty path if resolution fails.
 */
std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path);

/**
 * @brief Check whether the Salma plugin is installed at the given deploy path.
 *
 * Looks for `salma/mo2-salma.dll` and `mo2-salma.py` under @p deploy_path.
 *
 * @param deploy_path Path to the MO2 plugins directory.
 * @return `true` if both plugin files exist, `false` otherwise
 *         (including on exceptions such as permissions errors).
 */
bool plugin_installed_at(const std::filesystem::path& deploy_path);

}  // namespace mo2server

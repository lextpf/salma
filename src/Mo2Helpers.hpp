#pragma once

#include "Logger.hpp"
#include "Utils.hpp"

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
 * @fn crow::response json_response(int code, const nlohmann::json& j)
 * @brief can fail when native path bytes are not valid UTF-8.
 * @author Alex (https://github.com/lextpf)
 *
 * serialization uses strict UTF-8 validation and can raise
 * `nlohmann::json::type_error` for a native Windows path.
 *
 * @param code HTTP status code.
 * @param j value to serialize.
 * @return response with an `application/json` content type.
 */
inline crow::response json_response(int code, const nlohmann::json& j)
{
    crow::response res(code, j.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

/**
 * @fn std::string url_decode(const std::string& src)
 * @brief keeps malformed escapes and removes encoded null bytes.
 * @author Alex (https://github.com/lextpf)
 *
 * `%XX` becomes one byte, `+` becomes a space, malformed escapes remain, and
 * encoded null bytes are removed. literal null bytes remain. output is not
 * validated as UTF-8 or as a safe path.
 *
 * @param src encoded byte string.
 * @return decoded byte string.
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
/**
 * @struct HandleGuard
 * @brief owns one valid Win32 handle.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Mo2Helpers
 *
 * destruction closes a non-null handle. `release` transfers ownership. the type
 * is neither copyable nor movable.
 *
 * @warning never assign `INVALID_HANDLE_VALUE`; only null is treated as empty.
 */
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
 * @fn std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path)
 * @brief gives SALMA_DEPLOY_PATH precedence over derived paths.
 * @author Alex (https://github.com/lextpf)
 *
 * `SALMA_DEPLOY_PATH` wins when set. otherwise the function uses the parameter,
 * then `SALMA_MODS_PATH`, and derives `<mods>/../../MO2/plugins`. the result is
 * not canonicalized, created, or checked for existence.
 *
 * @param mo2_mods_path configured mods directory, or empty.
 * @return the derived directory, or an empty path when no usable input exists.
 */
std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path);

/**
 * @fn bool plugin_installed_at(const std::filesystem::path& deploy_path)
 * @brief requires both the DLL and Python entry point.
 * @author Alex (https://github.com/lextpf)
 *
 * this tests existence only. filesystem failures are logged and return `false`.
 *
 * @param deploy_path MO2 plugin directory, or empty.
 * @return `true` when both `salma/mo2-salma.dll` and `mo2-salma.py` exist.
 */
bool plugin_installed_at(const std::filesystem::path& deploy_path);

}  // namespace mo2server

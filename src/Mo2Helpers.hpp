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
 * @brief Build a crow::response with a JSON body and Content-Type header.
 * @ingroup Mo2Helpers
 *
 * The body is `j.dump()`: compact, no indentation, no trailing newline.
 * The header is `application/json` with no `charset` parameter.
 *
 * `dump()` runs with nlohmann's strict UTF-8 error handler, so a string
 * inside @p j that is not valid UTF-8 throws
 * `nlohmann::json::type_error`. That path is reachable: on Windows
 * `std::filesystem::path::string()` returns the native narrow encoding,
 * which is not guaranteed to be UTF-8. A handler that puts a path into a
 * response body and does not catch `std::exception` lets the throw
 * escape into Crow.
 *
 * @param code HTTP status code. Not validated.
 * @param j JSON body to serialize via `dump()`.
 * @return Crow response with `Content-Type: application/json`.
 * @throw nlohmann::json::type_error When @p j holds a string that is not
 *        valid UTF-8.
 */
inline crow::response json_response(int code, const nlohmann::json& j)
{
    crow::response res(code, j.dump());
    res.set_header("Content-Type", "application/json");
    return res;
}

/**
 * @brief Decode an `application/x-www-form-urlencoded` string.
 * @ingroup Mo2Helpers
 *
 * Implements the HTML form-encoding superset of RFC 3986 percent
 * decoding:
 *
 * - `%XX` sequences decode to bytes, hex digits in either case.
 * - `+` becomes a space. That is form encoding, not strict RFC 3986, so
 *   a caller decoding an RFC 3986 path component has to pre-filter `+`
 *   or use a strict decoder.
 * - Percent-encoded null bytes (`%00`) are dropped silently. A literal
 *   0x00 byte already in @p src is copied through unchanged, so a caller
 *   that hands the result to a C-string consumer has to screen it
 *   itself.
 * - A malformed `%XX` (too few characters, or a non-hex digit) is left
 *   in place verbatim.
 *
 * The result is a byte string. Decoded bytes are neither validated as
 * UTF-8 nor normalized, so the output can hold any byte. A caller that
 * builds a filesystem path from the result still needs its own
 * containment check; `get_fomod` and `delete_fomod` use
 * `mo2core::is_inside` for exactly that.
 *
 * Allocation aside, the decoder cannot fail: no input is rejected and no
 * error is reported.
 *
 * @param src URL-encoded input string.
 * @return Decoded string. Free of percent-encoded NULs, but not of a NUL
 *         the input carried literally.
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
 * @ingroup Mo2Helpers
 *
 * Whitespace is whatever `std::isspace` reports for the current C
 * locale, tested one byte at a time, so multi-byte characters are not
 * understood. The caller is the `meta.ini` reader in
 * Mo2FomodController.cpp, whose input is ASCII key/value text.
 *
 * @param s Input string.
 * @return A new string with leading/trailing whitespace removed. An
 *         all-whitespace input yields an empty string.
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
/**
 * @struct HandleGuard
 * @brief RAII owner for one Win32 HANDLE.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Mo2Helpers
 *
 * Closes `h` in the destructor when it is not null, so an early return
 * or an exception cannot leak the handle. Assign the handle straight to
 * the public member after the Win32 call that produced it.
 *
 * Ownership rules:
 *
 * - The guard closes exactly one handle, once.
 * - `release()` hands the handle back to the caller and clears the
 *   member, so the destructor then does nothing. Use it when the handle
 *   has to outlive the scope, such as a process handle kept for later
 *   polling.
 * - Copy is deleted and no move operations are declared, so the guard is
 *   neither copyable nor movable. It cannot be returned by value or
 *   stored in a container.
 * - Only a null `h` is skipped. `INVALID_HANDLE_VALUE` does not count as
 *   empty, so never assign it to the member.
 *
 * Windows only, and currently unreferenced: the two spawn sites
 * (Mo2PluginController.cpp and Mo2TestController.cpp) manage their
 * handles by hand because one of the handles has to survive the
 * function.
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
 * @brief Resolve the MO2 plugin deploy path from config/env.
 * @ingroup Mo2Helpers
 *
 * Reads two environment variables and does pure path arithmetic. It
 * touches no file and creates no directory.
 *
 * Resolution order:
 *
 * 1. `SALMA_DEPLOY_PATH` if non-empty, returned verbatim.
 * 2. The `mo2_mods_path` parameter if non-empty.
 * 3. `SALMA_MODS_PATH` if step 2 was empty.
 *
 * With a mods path from step 2 or 3, the deploy path is
 * `{mods_path.parent_path().parent_path()} / "MO2" / "plugins"`. The
 * walk climbs two levels, out of the mods directory and out of the MO2
 * directory that holds it, then descends back into `MO2/plugins`, which
 * is why the literal `MO2` segment reappears in the result. README.md
 * states the same rule as `<mods_path>/../../MO2/plugins`.
 *
 * Worked example for the standard layout:
 *
 * ```text
 * SALMA_DEPLOY_PATH unset
 * mo2_mods_path      = D:/Games/MO2/mods
 *   parent_path()   -> D:/Games/MO2
 *   parent_path()   -> D:/Games
 * result             = D:/Games/MO2/plugins
 *
 * plugin_installed_at() then probes:
 *   D:/Games/MO2/plugins/salma/mo2-salma.dll
 *   D:/Games/MO2/plugins/mo2-salma.py
 * ```
 *
 * Failure logs `[server] Cannot determine plugin deploy path` at error
 * level and returns an empty path. Two distinct inputs reach it:
 *
 * - Neither `SALMA_DEPLOY_PATH` nor a mods path (parameter or
 *   `SALMA_MODS_PATH`) is set.
 * - A mods path is set but has fewer than two parent levels, so the
 *   two-level walk yields an empty path. In practice that means a
 *   relative path with fewer than three components, such as `mods` or
 *   `MO2/mods`. An absolute path always keeps its root, so it never
 *   reaches this branch.
 *
 * The returned path is not validated: not canonicalized, not checked for
 * existence, not created. Callers probe it themselves, usually with
 * `plugin_installed_at`.
 *
 * @param mo2_mods_path Configured MO2 mods directory (may be empty).
 * @return Resolved deploy path, or an empty path on either failure
 *         above.
 */
std::filesystem::path resolve_deploy_path(const std::string& mo2_mods_path);

/**
 * @brief Check whether the Salma plugin is installed at the given deploy path.
 * @ingroup Mo2Helpers
 *
 * Reports installed only when both `<deploy_path>/salma/mo2-salma.dll`
 * and `<deploy_path>/mo2-salma.py` exist, so a half-deployed directory
 * reports false. Existence is the only test: neither file is opened,
 * hashed or version-checked, so a stale DLL still reports installed.
 *
 * An empty @p deploy_path short-circuits to false without touching the
 * filesystem. That is the normal result when `resolve_deploy_path`
 * failed.
 *
 * @param deploy_path Path to the MO2 plugins directory.
 * @return `true` if both plugin files exist, `false` otherwise. A
 *         filesystem exception (for example a permissions error) is
 *         caught, logged as a warning, and reported as `false`; it
 *         never propagates.
 */
bool plugin_installed_at(const std::filesystem::path& deploy_path);

}  // namespace mo2server

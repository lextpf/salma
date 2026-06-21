#pragma once

#include "Export.hpp"

#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @fn std::vector<std::string> default_allowed_origins()
 * @brief limits default browser access to four loopback origins.
 * @author Alex (https://github.com/lextpf)
 *
 * the list contains `localhost` and `127.0.0.1` on ports 5000 and 3000.
 *
 * @return four origin strings for the production and development servers.
 */
MO2_API std::vector<std::string> default_allowed_origins();

/**
 * @fn std::vector<std::string> parse_origin_list(std::string_view csv)
 * @brief retains duplicates and performs no origin validation.
 * @author Alex (https://github.com/lextpf)
 *
 * entries are trimmed but not validated or normalized. duplicates are retained.
 *
 * @param csv source text.
 * @return non-empty entries in document order.
 */
MO2_API std::vector<std::string> parse_origin_list(std::string_view csv);

/**
 * @fn bool origin_in_allowlist(const std::vector<std::string>&, std::string_view)
 * @brief uses byte-exact, case-sensitive origin matching.
 * @author Alex (https://github.com/lextpf)
 *
 * comparison is byte-exact and case-sensitive. an empty origin never matches.
 *
 * @param allowlist permitted origin strings.
 * @param origin request `Origin` value.
 * @return `true` when the complete value is present.
 */
MO2_API bool origin_in_allowlist(const std::vector<std::string>& allowlist,
                                 std::string_view origin);

/**
 * @fn bool is_state_changing(std::string_view method)
 * @brief recognizes only the fixed POST, PUT, DELETE, and PATCH set.
 * @author Alex (https://github.com/lextpf)
 *
 * matching is case-insensitive. only POST, PUT, DELETE, and PATCH return `true`.
 *
 * @param method HTTP method name.
 * @return `true` when the method is in the fixed state-changing set.
 * @warning add any new state-changing method to this set before routing it.
 */
MO2_API bool is_state_changing(std::string_view method);

/**
 * @fn bool constant_time_equals(std::string_view a, std::string_view b)
 * @brief compares equal-length strings without a data-dependent early exit.
 * @author Alex (https://github.com/lextpf)
 *
 * length mismatches return immediately. equal-length inputs compare every byte.
 * the guarantee applies to source-level control flow only.
 *
 * @param a first input.
 * @param b second input.
 * @return `true` when both inputs have identical bytes.
 */
MO2_API bool constant_time_equals(std::string_view a, std::string_view b);

/**
 * @class SecurityContext
 * @brief owns the process CSRF token and browser origin allowlist.
 * @author Alex (https://github.com/lextpf)
 * @ingroup SecurityContext
 *
 * construction reads `SALMA_ALLOWED_ORIGINS` once and uses the defaults when it
 * contains no entries. state is immutable after construction.
 *
 * ### :material-shield-lock: token security
 *
 * the 64-character lowercase token is regenerated at startup and is not persisted.
 * compare it with `constant_time_equals`.
 *
 * @warning `random_hex_string` uses MT19937 with one 32-bit seed. bind the server to
 *          loopback until the token uses a cryptographic generator.
 */
class MO2_API SecurityContext
{
public:
    /**
     * @fn SecurityContext& SecurityContext::instance()
     * @brief freezes the token and allowlist at first access.
     * @author Alex (https://github.com/lextpf)
     *
     * the first call fixes the token and allowlist for the process lifetime.
     *
     * @return the immutable process instance.
     */
    static SecurityContext& instance();

    /**
     * @fn const std::string& SecurityContext::csrf_token() const noexcept
     * @brief exposes one stable 64-character token for the process lifetime.
     * @author Alex (https://github.com/lextpf)
     *
     * @return a reference to 64 lowercase hexadecimal characters.
     */
    const std::string& csrf_token() const noexcept { return csrf_token_; }

    const std::vector<std::string>& allowed_origins() const noexcept { return allowed_origins_; }

    /**
     * @fn bool SecurityContext::is_origin_allowed(std::string_view origin) const
     * @brief delegates to byte-exact allowlist matching.
     * @author Alex (https://github.com/lextpf)
     *
     * @param origin request `Origin` value.
     * @return `true` after an exact match.
     */
    bool is_origin_allowed(std::string_view origin) const;

private:
    SecurityContext();
    SecurityContext(const SecurityContext&) = delete;
    SecurityContext& operator=(const SecurityContext&) = delete;

    std::string csrf_token_;
    std::vector<std::string> allowed_origins_;
};

}  // namespace mo2core

#pragma once

#include "Export.hpp"

#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @fn std::vector<std::string> default_allowed_origins()
 * @brief Limits default browser access to four loopback origins.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The list contains `localhost` and `127.0.0.1` on ports 5000 and 3000.
 *
 * @return Four origin strings for the production and development servers.
 */
MO2_API std::vector<std::string> default_allowed_origins();

/**
 * @fn std::vector<std::string> parse_origin_list(std::string_view csv)
 * @brief Retains duplicates and performs no origin validation.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Entries are trimmed but not validated or normalized. Duplicates are retained.
 *
 * @param csv Source text.
 * @return Non-empty entries in document order.
 */
MO2_API std::vector<std::string> parse_origin_list(std::string_view csv);

/**
 * @fn bool origin_in_allowlist(const std::vector<std::string>&, std::string_view)
 * @brief Uses byte-exact, case-sensitive origin matching.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Comparison is byte-exact and case-sensitive. An empty origin never matches.
 *
 * @param allowlist Permitted origin strings.
 * @param origin Request `Origin` value.
 * @return `true` when the complete value is present.
 */
MO2_API bool origin_in_allowlist(const std::vector<std::string>& allowlist,
                                 std::string_view origin);

/**
 * @fn bool is_state_changing(std::string_view method)
 * @brief Recognizes only the fixed POST, PUT, DELETE, and PATCH set.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Matching is case-insensitive. Only POST, PUT, DELETE, and PATCH return `true`.
 *
 * @param method HTTP method name.
 * @return `true` when the method is in the fixed state-changing set.
 * @warning Add any new state-changing method to this set before routing it.
 */
MO2_API bool is_state_changing(std::string_view method);

/**
 * @fn bool constant_time_equals(std::string_view a, std::string_view b)
 * @brief Compares equal-length strings without a data-dependent early exit.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Length mismatches return immediately. Equal-length inputs compare every byte.
 * The guarantee applies to source-level control flow only.
 *
 * @param a First input.
 * @param b Second input.
 * @return `true` when both inputs have identical bytes.
 */
MO2_API bool constant_time_equals(std::string_view a, std::string_view b);

/**
 * @class SecurityContext
 * @brief Owns the process CSRF token and browser origin allowlist.
 * @author Alex (<https://github.com/lextpf>)
 * @ingroup SecurityContext
 *
 * Construction reads `SALMA_ALLOWED_ORIGINS` once and uses the defaults when it
 * contains no entries. State is immutable after construction.
 *
 * ### :material-shield-lock: Token security
 *
 * The 64-character lowercase token is regenerated at startup and is not persisted.
 * Compare it with `constant_time_equals`.
 *
 * @warning `random_hex_string` uses MT19937 with one 32-bit seed. Bind the server to
 *          loopback until the token uses a cryptographic generator.
 */
class MO2_API SecurityContext
{
public:
    /**
     * @fn SecurityContext& SecurityContext::instance()
     * @brief Freezes the token and allowlist at first access.
     * @author Alex (<https://github.com/lextpf>)
     *
     * The first call fixes the token and allowlist for the process lifetime.
     *
     * @return The immutable process instance.
     */
    static SecurityContext& instance();

    /**
     * @fn const std::string& SecurityContext::csrf_token() const noexcept
     * @brief Exposes one stable 64-character token for the process lifetime.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return A reference to 64 lowercase hexadecimal characters.
     */
    const std::string& csrf_token() const noexcept { return csrf_token_; }

    /**
     * @fn const std::vector<std::string>& SecurityContext::allowed_origins() const noexcept
     * @brief Expose the startup allowlist without copying it.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @return A reference valid for the process instance lifetime.
     */
    const std::vector<std::string>& allowed_origins() const noexcept { return allowed_origins_; }

    /**
     * @fn bool SecurityContext::is_origin_allowed(std::string_view origin) const
     * @brief Delegates to byte-exact allowlist matching.
     * @author Alex (<https://github.com/lextpf>)
     *
     * @param origin Request `Origin` value.
     * @return `true` after an exact match.
     */
    bool is_origin_allowed(std::string_view origin) const;

private:
    /**
     * @fn SecurityContext::SecurityContext()
     * @brief Generate the process token and read the startup origin policy.
     * @author Alex (<https://github.com/lextpf>)
     */
    SecurityContext();
    SecurityContext(const SecurityContext&) = delete;
    SecurityContext& operator=(const SecurityContext&) = delete;

    std::string csrf_token_;
    std::vector<std::string> allowed_origins_;
};

}  // namespace mo2core

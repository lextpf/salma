#pragma once

#include "Export.h"

#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @brief Default Origin allowlist when SALMA_ALLOWED_ORIGINS is not set.
 *
 * Covers the dashboard production origin (`mo2-server`'s default 5000 port)
 * and the Vite dev server's default 3000 port, on both `localhost` and
 * `127.0.0.1`. Origins are compared as opaque strings against the request's
 * @c Origin header.
 *
 * @return A new vector of four origins.
 */
MO2_API std::vector<std::string> default_allowed_origins();

/**
 * @brief Parse a comma-separated origin list, trimming surrounding whitespace
 *        and dropping empty entries.
 *
 * Performs no scheme/host validation. The returned strings are compared
 * verbatim against incoming `Origin` headers, so callers should pass
 * exactly what a browser sends (e.g. `http://localhost:3000`, no trailing
 * slash).
 *
 * @param csv  Raw env-var value (or other comma-separated source).
 * @return A new vector of cleaned origin strings; empty if @p csv is empty
 *         or contains only whitespace and separators.
 */
MO2_API std::vector<std::string> parse_origin_list(std::string_view csv);

/**
 * @brief Exact-match origin check against an allowlist.
 *
 * @param allowlist  Allowed origins (typically the SecurityContext list).
 * @param origin     Request `Origin` header value.
 * @return @c true if @p origin appears verbatim in @p allowlist.
 *         An empty @p origin always returns false.
 */
MO2_API bool origin_in_allowlist(const std::vector<std::string>& allowlist,
                                 std::string_view origin);

/**
 * @brief HTTP methods that mutate server state (POST, PUT, DELETE, PATCH).
 *
 * Case-insensitive. GET, HEAD, and OPTIONS are treated as safe.
 *
 * @param method  HTTP method name as a string.
 * @return @c true for state-changing methods, @c false otherwise.
 */
MO2_API bool is_state_changing(std::string_view method);

/**
 * @brief Length-then-byte constant-time string comparison.
 *
 * Returns false immediately on length mismatch (the lengths of both
 * strings are non-secret in CSRF token comparison: the server token has
 * a fixed, public length). For equal-length inputs, every byte is XORed
 * and combined into a single accumulator, so the runtime is independent
 * of where the first differing byte lies. Used for CSRF token validation
 * to avoid leaking the matching prefix length via timing.
 *
 * @param a  First input.
 * @param b  Second input.
 * @return @c true iff the inputs have identical length and content.
 */
MO2_API bool constant_time_equals(std::string_view a, std::string_view b);

/**
 * @class SecurityContext
 * @brief Process-lifetime CSRF token and Origin allowlist for the HTTP server.
 *
 * Meyer's singleton initialized lazily on first access. The constructor
 * generates a 64-hex-char (256-bit) random CSRF token via
 * @ref random_hex_string and reads the @c SALMA_ALLOWED_ORIGINS env var
 * into the allowlist (falling back to @ref default_allowed_origins when
 * the env var is unset or empty after parsing).
 *
 * The token is regenerated on every server restart and never persisted.
 * The instance is read-only after construction; all public accessors are
 * thread-safe (no internal mutation).
 *
 * @note Lives in `mo2-core` so `salma_tests` (which links only
 *       `mo2-core`) can exercise the underlying free helpers without
 *       pulling in Crow.
 */
class MO2_API SecurityContext
{
public:
    /**
     * @brief Get the singleton SecurityContext.
     *
     * First call generates the token and parses the env var; subsequent
     * calls return the same instance.
     */
    static SecurityContext& instance();

    /**
     * @brief Return the CSRF token expected on state-changing requests.
     *
     * 64 lowercase hex chars (256 bits of entropy). Stable for the
     * lifetime of the process.
     */
    const std::string& csrf_token() const noexcept { return csrf_token_; }

    /**
     * @brief Return the parsed Origin allowlist.
     */
    const std::vector<std::string>& allowed_origins() const noexcept { return allowed_origins_; }

    /**
     * @brief Check a request's @c Origin header against the allowlist.
     *
     * Convenience wrapper around @ref origin_in_allowlist.
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

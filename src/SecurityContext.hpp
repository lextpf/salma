#pragma once

#include "Export.hpp"

#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @brief Default Origin allowlist when SALMA_ALLOWED_ORIGINS is not set.
 * @ingroup SecurityContext
 *
 * Covers the dashboard production origin (`mo2-server`'s default 5000 port) and
 * the Vite dev server's default 3000 port, on both `localhost` and
 * `127.0.0.1`. Origins are compared as opaque strings against the request's
 * @c Origin header, so the two host spellings need separate entries; they are
 * not interchangeable at comparison time.
 *
 * @return A new vector of four origins, in the order 5000/localhost,
 *         5000/127.0.0.1, 3000/localhost, 3000/127.0.0.1. The order does not
 *         affect matching.
 */
MO2_API std::vector<std::string> default_allowed_origins();

/**
 * @brief Parse a comma-separated origin list, trimming surrounding whitespace
 *        and dropping empty entries.
 * @ingroup SecurityContext
 *
 * No scheme or host validation. The returned strings are compared verbatim
 * against incoming `Origin` headers, so pass exactly what a browser sends:
 * `http://localhost:3000`, no trailing slash. Duplicates are preserved and
 * harmless, because matching stops at the first hit.
 *
 * @param csv  Raw env-var value (or other comma-separated source).
 * @return A new vector of cleaned origin strings; empty if @p csv is empty
 *         or contains only whitespace and separators.
 */
MO2_API std::vector<std::string> parse_origin_list(std::string_view csv);

/**
 * @brief Exact-match origin check against an allowlist.
 * @ingroup SecurityContext
 *
 * The comparison is byte-exact and case-sensitive. It does no scheme, host or
 * port normalization, so `http://localhost:3000` and `http://LOCALHOST:3000`
 * are different origins.
 *
 * @param allowlist  Allowed origins (typically the SecurityContext list).
 * @param origin     Request `Origin` header value.
 * @return @c true if @p origin appears verbatim in @p allowlist.
 *         An empty @p origin always returns false, including when the
 *         allowlist itself is empty.
 */
MO2_API bool origin_in_allowlist(const std::vector<std::string>& allowlist,
                                 std::string_view origin);

/**
 * @brief HTTP methods that mutate server state (POST, PUT, DELETE, PATCH).
 * @ingroup SecurityContext
 *
 * Case-insensitive. GET, HEAD and OPTIONS are treated as safe, and so is any
 * method the list does not name. A new state-changing method must be added
 * here, or it bypasses the CSRF check.
 *
 * @param method  HTTP method name as a string.
 * @return @c true for state-changing methods, @c false otherwise.
 */
MO2_API bool is_state_changing(std::string_view method);

/**
 * @brief Length-then-byte constant-time string comparison.
 * @ingroup SecurityContext
 *
 * Returns false immediately on a length mismatch. Length is not a secret in
 * CSRF token comparison: the server token has a fixed, public length. For
 * equal-length inputs every byte is XORed into one accumulator, so the runtime
 * does not depend on where the first differing byte lies and the matching
 * prefix length does not leak through timing.
 *
 * The guarantee is about source-level control flow. It is not a defence against
 * a compiler or CPU that reintroduces a data-dependent early exit.
 *
 * @param a  First input.
 * @param b  Second input.
 * @return @c true iff the inputs have identical length and content.
 */
MO2_API bool constant_time_equals(std::string_view a, std::string_view b);

/**
 * @class SecurityContext
 * @brief Process-lifetime CSRF token and Origin allowlist for the HTTP server.
 * @author Alex (https://github.com/lextpf)
 * @ingroup SecurityContext
 *
 * Meyer's singleton initialized lazily on first access. Everything it holds is
 * decided in the constructor and never written again.
 *
 * This page is authoritative for the token and the allowlist themselves.
 * `mo2server::SecurityMiddleware` is authoritative for how a request is judged
 * against them and for the CORS response headers.
 *
 * ## :material-shield-check: CSRF token
 *
 * The constructor generates a 64-character lowercase hex token with
 * `random_hex_string`. The server issues it from `/api/csrf-token` and
 * requires it in the `X-Salma-Csrf` header on every state-changing request.
 * Compare it with `constant_time_equals`, never with `==`.
 *
 * The token is regenerated on every server restart and never persisted, so a
 * restart invalidates any token a client is still holding.
 *
 * @warning 64 hex characters wide, but not 256 bits of entropy.
 *   `random_hex_string` draws every nibble from a `std::mt19937` seeded once
 *   from a single 32-bit `std::random_device` draw, so the whole token is a
 *   function of at most 2^32 seeds and the generator is not cryptographic.
 *   Treat the effective strength as about 32 bits. That is an accepted risk for
 *   a dashboard bound to loopback. Replace the generator with a platform CSPRNG
 *   (`BCryptGenRandom` on Windows) before exposing the server on any other
 *   interface.
 *
 * ## :material-api: Origin allowlist
 *
 * The constructor reads the `SALMA_ALLOWED_ORIGINS` environment variable and
 * parses it with `parse_origin_list`. It falls back to
 * `default_allowed_origins` when the variable is unset, empty, or parses to
 * nothing, so the allowlist is never empty. Matching is the byte-exact test in
 * `origin_in_allowlist`.
 *
 * ## :material-help: Thread Safety
 *
 * instance() is serialized by the C++11 magic static. The instance is read-only
 * after construction, so every accessor is safe to call concurrently from any
 * Crow request handler. `csrf_token()` and `allowed_origins()` return
 * references into the singleton, valid for the whole process.
 *
 * @note SecurityContext.cpp is compiled into the `salma-support` static library
 *       alongside Utils.cpp and Logger.cpp, the half of the C++ that does not
 *       depend on Crow. `salma_tests` links that library, so
 *       tests/security_context_test.cpp exercises the free helpers above
 *       without pulling Crow into the test binary.
 */
class MO2_API SecurityContext
{
public:
    /**
     * @brief Get the singleton SecurityContext.
     *
     * The first call generates the token and parses the environment variable.
     * `SALMA_ALLOWED_ORIGINS` is read once, so changing it afterwards has no
     * effect on a running server.
     *
     * @return Reference to the process-wide instance, valid until process exit.
     */
    static SecurityContext& instance();

    /**
     * @brief Return the CSRF token expected on state-changing requests.
     *
     * 64 lowercase hex characters, stable for the lifetime of the process. The
     * warning in the class description applies: the effective entropy is
     * bounded by a 32-bit seed, not by the token's width.
     *
     * @return Reference to the stored token. Never empty.
     */
    const std::string& csrf_token() const noexcept { return csrf_token_; }

    /**
     * @brief Return the parsed Origin allowlist.
     * @return Reference to the stored allowlist. Never empty: the constructor
     *         substitutes `default_allowed_origins` when parsing yields
     *         nothing.
     */
    const std::vector<std::string>& allowed_origins() const noexcept { return allowed_origins_; }

    /**
     * @brief Check a request's @c Origin header against the allowlist.
     *
     * Convenience wrapper around `origin_in_allowlist` using this instance's
     * allowlist. An empty @p origin is rejected.
     *
     * @param origin  Request `Origin` header value.
     * @return @c true when the origin is allowed.
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

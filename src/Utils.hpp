#pragma once

#include "Export.hpp"
#include "Types.hpp"

#include <array>
#include <expected>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @struct EnumStringMap
 * @brief Bidirectional constexpr enum to string mapping.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Utils
 *
 * Both directions are a linear scan over `entries`, evaluated at compile time
 * when the arguments are constant. Matching is exact and case-sensitive.
 * Neither direction reports a miss: each returns the configured default, so a
 * caller that must tell "absent" from "mapped to the default value" has to
 * check the input itself.
 *
 * @tparam Enum  The enum type to map.
 * @tparam N     Number of entries in the mapping table.
 */
template <typename Enum, std::size_t N>
struct EnumStringMap
{
    std::array<std::pair<Enum, std::string_view>, N> entries;  ///< Enum/string pairs.
    Enum default_value;  ///< Returned by from_string() on a lookup miss.
    /// Returned by to_string() on a lookup miss. Points at a string literal
    /// with static storage duration, so the view never dangles.
    std::string_view default_string = "Unknown";

    /**
     * @brief Look up an enum value by its string representation.
     * @param s  String to search for (exact, case-sensitive match).
     * @return The matching enum value, or `default_value` if not found.
     */
    [[nodiscard]] constexpr Enum from_string(std::string_view s) const noexcept
    {
        for (const auto& [e, str] : entries)
            if (str == s)
                return e;
        return default_value;
    }

    /**
     * @brief Look up the string representation of an enum value.
     * @param e  Enum value to search for.
     * @return The matching string, or `default_string` if not found. The view
     *         points into `entries`, so it stays valid for as long as the map.
     */
    [[nodiscard]] constexpr std::string_view to_string(Enum e) const noexcept
    {
        for (const auto& [val, str] : entries)
            if (val == e)
                return str;
        return default_string;
    }
};

/**
 * @brief Variable template providing the canonical EnumStringMap for each enum type.
 * @ingroup Utils
 *
 * Specialize this for each enum that supports bidirectional string conversion.
 * The specialization must be visible at the point of use: otherwise the caller
 * silently gets the empty primary template, which converts every value to
 * "Unknown" and parses every string back to the zero-initialized value. There
 * is no compile error for that mistake.
 *
 * ```cpp
 * template <>
 * inline constexpr auto enum_map<MyEnum> = EnumStringMap<MyEnum, 2>{
 *     std::array<std::pair<MyEnum, std::string_view>, 2>{{
 *         {MyEnum::A, "A"},
 *         {MyEnum::B, "B"},
 *     }},
 *     MyEnum::A,  // default on lookup miss
 * };
 * ```
 */
template <typename Enum>
inline constexpr auto enum_map = EnumStringMap<Enum, 0>{};

/**
 * @brief Parse a string into an enum value using the registered EnumStringMap.
 * @ingroup Utils
 *
 * An unrecognized string is not an error; it yields the map's default value.
 *
 * @tparam Enum  The enum type. An `enum_map` specialization must be visible.
 * @param s  String to look up (exact, case-sensitive match).
 * @return Matching enum value, or the map's default on miss.
 */
template <typename Enum>
[[nodiscard]] constexpr Enum parse_enum(std::string_view s) noexcept
{
    return enum_map<Enum>.from_string(s);
}

/**
 * @brief Convert an enum value to its string representation.
 * @ingroup Utils
 *
 * @tparam Enum  The enum type. An `enum_map` specialization must be visible.
 * @param e  Enum value to look up.
 * @return Matching string, or "Unknown" on miss.
 */
template <typename Enum>
[[nodiscard]] constexpr std::string_view enum_to_string(Enum e) noexcept
{
    return enum_map<Enum>.to_string(e);
}

/// PluginType map: the five FOMOD `<type>` spellings, defaulting to `Optional`
/// on a lookup miss, which is also the engine IR's default plugin type.
template <>
inline constexpr auto enum_map<PluginType> = EnumStringMap<PluginType, 5>{
    std::array<std::pair<PluginType, std::string_view>, 5>{{
        {PluginType::Required, "Required"},
        {PluginType::Recommended, "Recommended"},
        {PluginType::Optional, "Optional"},
        {PluginType::NotUsable, "NotUsable"},
        {PluginType::CouldBeUsable, "CouldBeUsable"},
    }},
    PluginType::Optional,
};

/**
 * @brief Lowercase a string.
 * @ingroup Utils
 *
 * Each character is cast to `unsigned char` before reaching `std::tolower`,
 * which is undefined for negative `int` arguments when `char` is signed (the
 * MSVC default). Without the cast, high-bit bytes from UTF-8 paths trigger that
 * UB.
 *
 * @param s  Input string.
 * @return A new string with each ASCII letter lowercased. Under the default "C"
 *         locale only `A` to `Z` change and every byte >= 0x80 is left alone,
 *         so this is a byte-level lowercaser, not a Unicode case-folder. That
 *         locale is a precondition rather than a property of the function:
 *         `std::tolower` follows the process's `LC_CTYPE`, so a `setlocale`
 *         call anywhere in the process could start folding high bytes and
 *         change the result for UTF-8 paths. No salma code calls `setlocale`.
 * @throw Does not throw.
 */
MO2_API std::string to_lower(const std::string& s);

/**
 * @brief Normalize an archive/mod path.
 * @ingroup Utils
 *
 * The pipeline is applied in this order:
 *
 * ```mermaid
 * flowchart LR
 *     A[Raw path] --> B[lowercase]
 *     B --> C[backslash -> forward slash]
 *     C --> D[strip leading './' and '/']
 *     D --> E[strip trailing '/']
 *     E --> F[collapse '//' -> '/']
 *     F --> G[drop '.' and '..' segments]
 *     G --> H[Normalized path]
 * ```
 *
 * The `.` and `..` removal is a syntactic strip, not a filesystem resolution: a
 * `..` segment is dropped rather than applied to the segment before it, so
 * `a/b/../c` normalizes to `a/b/c` and not to `a/c`. Two inputs that resolve to
 * the same location on disk can therefore normalize to different strings. Treat
 * the output as a comparison key, not as a resolved path.
 *
 * What the strip does guarantee is that no `..` survives, so a normalized path
 * can never climb above the directory it is later joined to.
 * `is_safe_destination` depends on exactly that property.
 *
 * @param p  Raw path string (e.g. from an archive entry or FOMOD node).
 * @return Cleaned path string. An input consisting only of separators, `.`
 *         and `..` segments returns the empty string.
 * @post Output is lowercase, uses forward slashes only, has no
 *   leading/trailing slashes, no repeated `/`, and no `.` or `..`
 *   path segments.
 * @throw Does not throw.
 */
MO2_API std::string normalize_path(const std::string& p);

/**
 * @brief Generate a random hex string of the given length.
 * @ingroup Utils
 *
 * The generator is `thread_local`: each thread seeds its own `std::mt19937` on
 * first use. That removes cross-thread contention, and it means two threads
 * produce independent streams.
 *
 * @param length  Number of hex characters (default 12). A length of 0 returns
 *                the empty string.
 * @return A lowercase hex string of exactly @p length characters, drawn from
 *         `0123456789abcdef`.
 * @throw Only what `std::string` allocation throws.
 *
 * @warning Not cryptographically secure. Each thread's `std::mt19937` is seeded
 *   from a single 32-bit `std::random_device` draw, so however many characters
 *   are requested the whole output stream is a function of at most 2^32 seeds,
 *   and mt19937 state can be reconstructed from its output.
 *   `mo2core::SecurityContext` uses this for the CSRF token, an accepted risk
 *   for a dashboard bound to loopback. That is not a licence to use it for
 *   secrets in general: anything that must resist an attacker needs a platform
 *   CSPRNG (`BCryptGenRandom` on Windows).
 */
MO2_API std::string random_hex_string(size_t length = 12);

/**
 * @brief Map a FOMOD plugin type name string to its PluginType enum value.
 * @ingroup Utils
 *
 * Thin wrapper over `parse_enum<PluginType>`: exact, case-sensitive match, and
 * an unrecognized name is not an error.
 *
 * @param type_name  FOMOD type name, for example "Required" or "Recommended".
 * @return Corresponding PluginType, or PluginType::Optional if unrecognized.
 * @throw Does not throw.
 */
MO2_API PluginType parse_plugin_type_string(const std::string& type_name);

/**
 * @brief Map a PluginType enum value to its FOMOD type name string.
 * @ingroup Utils
 *
 * No caller today; only parse_plugin_type_string() is exercised. Kept so the
 * mapping stays bidirectional.
 *
 * @param type  PluginType enum value.
 * @return String representation such as "Required", or "Unknown" on a miss. The
 *         view points at a string literal and never dangles.
 * @throw Does not throw.
 */
MO2_API std::string_view plugin_type_to_string(PluginType type);

/**
 * @brief FNV-1a 64-bit hash.
 * @ingroup Utils
 *
 * Recurrence over the input bytes $b_0 \dots b_{n-1}$, with the result $h_n$:
 *
 * - $h_0 = \mathtt{0xCBF29CE484222325}$
 * - $h_{i+1} = (h_i \oplus b_i) \times \mathtt{0x100000001B3}$
 *
 * The multiply wraps modulo $2^{64}$. Unsigned overflow is defined, so the wrap
 * is intended, not a bug. Each byte is fed in as `unsigned char`, which makes
 * the hash independent of whether `char` is signed. The order is byte-stream,
 * not bit-stream.
 *
 * No direct caller in C++ beyond the `_h` literal below. The engine's FNV-1a
 * must produce identical values for identical bytes, so this stays as the
 * readable statement of the recurrence both sides implement.
 *
 * @param data  Pointer to the byte sequence to hash. Must be non-null when
 *              @p size is greater than 0.
 * @param size  Number of bytes to hash.
 * @return 64-bit FNV-1a hash value. A zero-length input returns the offset
 *         basis unchanged.
 * @throw Does not throw.
 */
MO2_API constexpr uint64_t fnv1a_hash(const char* data, size_t size)
{
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < size; ++i)
    {
        hash ^= static_cast<uint64_t>(static_cast<unsigned char>(data[i]));
        hash *= 1099511628211ULL;
    }
    return hash;
}

/**
 * @brief Compile-time string hash literal for switch-case dispatch.
 * @ingroup Utils
 *
 * `consteval`, so a use that is not a constant expression fails to compile
 * rather than falling back to a runtime hash.
 *
 * ```cpp
 * case "flagDependency"_h: ...
 * ```
 *
 * Any dispatch table built on this literal must be guarded with
 * no_hash_collisions(): a collision would otherwise route two different keys to
 * the same case label, silently. Nothing uses the literal today.
 *
 * @param s  Pointer to the string literal.
 * @param n  Length of the string literal, excluding the terminating null.
 * @return Compile-time FNV-1a hash of the string.
 */
consteval uint64_t operator""_h(const char* s, size_t n)
{
    return fnv1a_hash(s, n);
}

/**
 * @brief Compile-time collision checker for hash dispatch tables.
 * @ingroup Utils
 *
 * Compares every pair of keys, so the cost is quadratic in @p N. The check runs
 * at compile time over small tables, so that is affordable.
 *
 * ```cpp
 * static_assert(no_hash_collisions(std::array{ "a"sv, "b"sv, "c"sv }));
 * ```
 *
 * The companion guard for the `_h` literal. No static_assert uses it today.
 *
 * @tparam N  Number of keys to check.
 * @param keys  The dispatch keys.
 * @return `true` when all @p N keys hash to distinct values.
 */
template <std::size_t N>
consteval bool no_hash_collisions(const std::array<std::string_view, N>& keys)
{
    for (std::size_t i = 0; i < N; ++i)
        for (std::size_t j = i + 1; j < N; ++j)
            if (fnv1a_hash(keys[i].data(), keys[i].size()) ==
                fnv1a_hash(keys[j].data(), keys[j].size()))
                return false;
    return true;
}

/**
 * @struct HashDispatch
 * @brief Compile-time hash dispatch table: maps string keys to values via FNV-1a.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Utils
 *
 * Entries store the precomputed hash, not the key, so a lookup compares 64-bit
 * integers instead of strings. The table cannot detect a collision at lookup
 * time and returns the first entry with a matching hash, so verify the key set
 * with no_hash_collisions() at compile time before relying on it.
 *
 * The third piece of the `_h` toolkit, with the literal and the collision
 * checker. Nothing instantiates it today.
 *
 * @tparam T  Value type stored in each entry.
 * @tparam N  Number of entries in the dispatch table.
 */
template <typename T, std::size_t N>
struct HashDispatch
{
    /// A single hash-to-value entry.
    struct Entry
    {
        uint64_t hash;  ///< Precomputed FNV-1a hash of the key.
        T value;        ///< Value associated with the key.
    };
    std::array<Entry, N> entries;  ///< The dispatch table entries.

    /**
     * @brief Look up a value by its string key.
     * @param key  String to hash and search for.
     * @return A copy of the matching value, or `std::nullopt` on miss.
     */
    [[nodiscard]] constexpr std::optional<T> lookup(std::string_view key) const noexcept
    {
        uint64_t h = fnv1a_hash(key.data(), key.size());
        for (const auto& [eh, ev] : entries)
            if (eh == h)
                return ev;
        return std::nullopt;
    }
};

/**
 * @brief Strip leading slashes and "./" from FOMOD destinations so they are
 *   safe to join with a mod-root directory path.
 * @ingroup Utils
 *
 * FOMOD destinations are relative to the mod root. A value of `\` or `/` means
 * "the root", not an absolute filesystem path, so leading separators are
 * removed rather than treated as an anchor. Both separator spellings are
 * accepted and the case of the input is preserved.
 *
 * No C++ caller; the live path runs in the engine.
 *
 * @param destination  Raw FOMOD destination string (taken by value).
 * @return Cleaned destination with leading separators and a leading "./" (or
 *         its backslash form) removed. A destination made only of separators
 *         returns the empty string, which means the mod root.
 * @throw Only what `std::string` operations throw.
 */
MO2_API std::string normalize_destination_for_join(std::string destination);

/**
 * @brief Resolve a \<file\>/\<folder\> node's destination, handling empty destinations
 *   and trailing-slash directory semantics, then normalize for filesystem join.
 * @ingroup Utils
 *
 * Two FOMOD conventions apply, both only to `<file>` nodes: an empty
 * destination means "keep the source file name", and a destination ending in a
 * separator means "a directory", so the source file name is appended to it. For
 * `<folder>` nodes @p raw_destination is carried through as-is. Every result
 * then passes through normalize_destination_for_join().
 *
 * No C++ caller; the live path runs in the engine.
 *
 * @param source           Source path from the FOMOD node.
 * @param raw_destination  Raw destination attribute value (may be empty).
 * @param is_file          True for \<file\> nodes, false for \<folder\> nodes.
 * @return Normalized destination path ready for filesystem join.
 * @throw Only what `std::string` operations throw.
 */
MO2_API std::string resolve_file_destination(const std::string& source,
                                             const std::string& raw_destination,
                                             bool is_file);

/**
 * @brief Reject destination paths that would escape the mod directory via traversal
 *   or absolute paths.
 * @ingroup Utils
 *
 * The candidate goes through normalize_path() first, which drops every `.` and
 * `..` segment, so no traversal sequence survives into the value that is
 * tested. What remains to reject is an absolute path (`/etc/passwd`) or a
 * Windows drive-qualified path (`C:/...`).
 *
 * Two inputs are accepted despite carrying no destination: the empty string,
 * and a string of only `.` and `..` segments, which normalizes to empty. Both
 * mean "the mod root", which is inside the mod directory and therefore safe.
 *
 * No C++ caller; the live guards run in the engine.
 *
 * @param dest  Destination path to validate.
 * @return `true` if the destination is safe (no traversal or absolute path).
 * @throw Only what `std::string` allocation throws.
 */
MO2_API bool is_safe_destination(const std::string& dest);

/**
 * @brief Reject mod-name strings that are unsafe to use as a single directory
 *   component under a mods root.
 * @ingroup Utils
 *
 * Accepts only single-segment, non-empty, non-reserved names. The upload
 * endpoint validates the `modName` form field with this before joining it to
 * the configured mods directory.
 *
 * Rejection rules, checked in this order:
 *   - empty
 *   - first or last character is whitespace (which also rejects a
 *     whitespace-only name, and " MyMod")
 *   - contains '/' or '\\'
 *   - parses as an absolute path (drive letter, leading separator)
 *   - equals "." or ".."
 *   - ends with '.'
 *   - lowercase stem (everything before the final '.') matches a Windows
 *     reserved device name: CON, PRN, AUX, NUL, COM1-9, LPT1-9
 *
 * The whitespace and trailing-'.' rules exist because `CreateFile` silently
 * strips those characters, so the directory Windows creates would not match the
 * name the user supplied.
 *
 * @param name  Candidate mod name.
 * @return `true` if @p name is a safe single-component directory name.
 * @throw Only what `std::string` allocation throws.
 */
MO2_API bool is_safe_mod_name(const std::string& name);

/**
 * @brief Validate that @p child is inside @p parent, or is @p parent itself.
 * @ingroup Utils
 *
 * Both paths go through `weakly_canonical` first, so the comparison survives
 * symlinks and non-existent trailing components. The test is then the relative
 * path from @p parent to @p child, accepted when it is non-empty and does not
 * begin with `..`.
 *
 * Two boundary cases follow from that predicate and matter wherever this is
 * used as a security guard:
 *
 * - **Equal paths return `true`.** Equal canonical paths relativize to `.`,
 *   which passes both conditions. Add an equality test where an operation must
 *   not target the container directory itself.
 * - **A child whose first component starts with two dots returns `false`.**
 *   `parent/..foo` relativizes to `..foo`, which begins with `..`, so a
 *   legitimate child is rejected. A known false negative that fails closed.
 *
 * @param parent  The directory that should contain the child.
 * @param child   The path to test.
 * @return `true` if @p child resolves to a location inside @p parent, or to
 *         @p parent itself.
 * @note `weakly_canonical` errors are silently treated as `false`. This is
 *       defensive but worth knowing when debugging spurious 403/400 responses
 *       caused by transient I/O failures during canonicalization.
 * @throw Does not throw on filesystem errors: both canonicalization calls use
 *        the `error_code` overload and report failure as `false`.
 */
MO2_API bool is_inside(const std::filesystem::path& parent, const std::filesystem::path& child);

/**
 * @brief Directory of the host executable (`GetModuleFileNameW(nullptr, ...)`).
 * @ingroup Utils
 *
 * Use this for resources tied to a specific executable, such as `salma.json`
 * next to `mo2-server.exe` or the dashboard's `web/dist` tree.
 *
 * Falls back to `std::filesystem::current_path()` when the Win32 lookup fails
 * or the platform is not Windows. The fallback keeps the function portable, at
 * the cost of silently handing a non-Windows caller the process working
 * directory instead.
 *
 * **Decision rule: executable vs module directory**
 *
 * - The resource follows the running executable (config files, dashboard
 *   assets) -> `executable_directory()`.
 * - The resource follows the binary that owns the calling code
 *   (`mo2-salma.dll`'s logs, the bundled `7z.dll` it loads) ->
 *   `module_directory(&YourSymbol)`.
 *
 * Under MO2 the host executable is `ModOrganizer.exe`, so
 * `executable_directory()` points at MO2's install root, not at the salma
 * plugin folder. Every salma-owned resource resolves through
 * `module_directory()` for that reason.
 *
 * @return Absolute path to the directory containing the running executable.
 *         The path is not cached; each call repeats the lookup.
 */
MO2_API std::filesystem::path executable_directory();

/**
 * @brief Directory of the module containing @p anchor.
 * @ingroup Utils
 *
 * On Windows this is the directory of the DLL or EXE the address lives in,
 * whatever the host process's working directory or the host executable's
 * location. Use it for resources owned by the binary the calling code is in
 * (logs, a bundled `7z.dll` next to `mo2-salma.dll`).
 *
 * Falls back to `std::filesystem::current_path()` when the lookup fails or the
 * platform is not Windows.
 *
 * The module handle is queried without incrementing its reference count, so
 * this call does not keep the module loaded.
 *
 * @param anchor Address inside the module to query (a function pointer
 *               to any symbol defined in that module is sufficient).
 * @return Absolute path to the module's containing directory. The path is not
 *         cached; each call repeats the lookup.
 * @see executable_directory() for the decision rule between the two.
 */
MO2_API std::filesystem::path module_directory(const void* anchor);

/**
 * @brief Typed error propagation alias.
 * @ingroup Utils
 *
 * `std::expected<T, std::string>`. The error channel carries a human-readable
 * message rather than an error code, so it holds text a caller can log or
 * return to the dashboard.
 *
 * The C++ spelling of the `Result` convention the engine follows. No C++ caller
 * uses it today.
 */
template <typename T>
using Result = std::expected<T, std::string>;

}  // namespace mo2core

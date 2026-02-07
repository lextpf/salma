#pragma once

#include "Export.h"
#include "Types.h"

#include <array>
#include <expected>
#include <filesystem>
#include <optional>
#include <pugixml.hpp>
#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/**
 * @brief Bidirectional constexpr enum <-> string mapping.
 * @tparam Enum  The enum type to map.
 * @tparam N     Number of entries in the mapping table.
 */
template <typename Enum, std::size_t N>
struct EnumStringMap
{
    std::array<std::pair<Enum, std::string_view>, N> entries; /**< Enum/string pairs. */
    Enum default_value;                          /**< Returned by from_string() on lookup miss. */
    std::string_view default_string = "Unknown"; /**< Returned by to_string() on lookup miss. */

    /**
     * @brief Look up an enum value by its string representation.
     * @param s  String to search for (exact match).
     * @return The matching enum value, or @c default_value if not found.
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
     * @return The matching string, or @c default_string if not found.
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
 *
 * Specialize this for each enum that supports bidirectional string conversion.
 * The primary template provides an empty map (always returns the zero-initialized
 * enum / "Unknown").
 *
 * @code
 * template <>
 * inline constexpr auto enum_map<MyEnum> = EnumStringMap<MyEnum, 2>{
 *     std::array<std::pair<MyEnum, std::string_view>, 2>{{
 *         {MyEnum::A, "A"},
 *         {MyEnum::B, "B"},
 *     }},
 *     MyEnum::A,  // default on lookup miss
 * };
 * @endcode
 */
template <typename Enum>
inline constexpr auto enum_map = EnumStringMap<Enum, 0>{};

/**
 * @brief Parse a string into an enum value using the registered EnumStringMap.
 * @tparam Enum  The enum type (must have an @c enum_map specialization visible).
 * @param s  String to look up.
 * @return Matching enum value, or the map's default on miss.
 */
template <typename Enum>
[[nodiscard]] constexpr Enum parse_enum(std::string_view s) noexcept
{
    return enum_map<Enum>.from_string(s);
}

/**
 * @brief Convert an enum value to its string representation.
 * @tparam Enum  The enum type (must have an @c enum_map specialization visible).
 * @param e  Enum value to look up.
 * @return Matching string, or "Unknown" on miss.
 */
template <typename Enum>
[[nodiscard]] constexpr std::string_view enum_to_string(Enum e) noexcept
{
    return enum_map<Enum>.to_string(e);
}

/** PluginType enum map specialization. */
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
 *
 * Each character is cast to `unsigned char` before being passed to
 * `std::tolower`. The cast is required because `std::tolower(int)`
 * is undefined for negative values when `char` is signed (the MSVC
 * default), and high-bit bytes from UTF-8 paths would otherwise
 * trigger that UB.
 *
 * @param s  Input string.
 * @return A new string with each ASCII character converted to its
 *         lowercase form. Bytes outside the ASCII range are left
 *         unchanged (this is a byte-level lowercaser, not a Unicode
 *         case-folder).
 * @throw Does not throw.
 */
MO2_API std::string to_lower(const std::string& s);

/**
 * @brief Normalize an archive/mod path.
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
 * The `.` / `..` removal is a syntactic strip, not a filesystem
 * resolution: there is no canonicalization against an actual
 * directory. Two callers that pass `a/b/../c` and `a/c` always get
 * the same normalized output regardless of whether `a/b` exists.
 *
 * @param p  Raw path string (e.g. from an archive entry or FOMOD node).
 * @return Cleaned path string.
 * @post Output is lowercase, uses forward slashes only, has no
 *   leading/trailing slashes, no repeated `/`, and no `.` or `..`
 *   path segments.
 * @throw Does not throw.
 */
MO2_API std::string normalize_path(const std::string& p);

/**
 * @brief Generate a random hex string of the given length using a thread-local RNG.
 * @param length  Number of hex characters (default 12).
 * @return A lowercase hex string of exactly @p length characters.
 */
MO2_API std::string random_hex_string(size_t length = 12);

/**
 * @brief Respect the FOMOD `order` attribute on installSteps, optionalFileGroups, and plugins.
 *
 * Values: "Ascending" (default, alphabetical by name), "Descending", "Explicit" (document order).
 *
 * When two nodes share the same `name` attribute under Ascending/Descending,
 * their relative order is unspecified (`std::ranges::sort` on equal keys is
 * not stable).
 *
 * @param parent      XML node whose `order` attribute is read.
 * @param child_name  Tag name of the child nodes to collect and sort.
 * @return Sorted vector of child nodes.
 */
MO2_API std::vector<pugi::xml_node> get_ordered_nodes(const pugi::xml_node& parent,
                                                      const char* child_name);

/**
 * @brief Parse an XML boolean attribute using XML Schema semantics.
 *
 * Returns true for "true"/"1" (case-insensitive), false otherwise.
 * A null/missing attribute returns false.
 * @param attr  The XML attribute to evaluate.
 * @return @c true if the attribute value is "true" or "1".
 */
MO2_API bool xml_bool_attribute_true(const pugi::xml_attribute& attr);

/**
 * @brief Map a FOMOD plugin type name string to its PluginType enum value.
 * @param type_name  FOMOD type name (e.g. "Required", "Recommended").
 * @return Corresponding PluginType, or PluginType::Optional if unrecognized.
 */
MO2_API PluginType parse_plugin_type_string(const std::string& type_name);

/**
 * @brief Map a PluginType enum value to its FOMOD type name string.
 * @param type  PluginType enum value.
 * @return String representation (e.g. "Required"), or "Unknown" if not found.
 */
MO2_API std::string_view plugin_type_to_string(PluginType type);

/**
 * @brief FNV-1a 64-bit hash.
 *
 * Recurrence:
 *
 * \f[ h_0 = \mathtt{0xCBF29CE484222325} \f]
 * \f[ h_{i+1} = (h_i \oplus b_i) \times \mathtt{0x100000001B3} \f]
 *
 * Each byte is fed in as `unsigned char`, so the hash is negative-char-safe.
 * Order is byte-stream, not bit-stream.
 *
 * @param data  Pointer to the byte sequence to hash.
 * @param size  Number of bytes to hash.
 * @return 64-bit FNV-1a hash value.
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
 *
 * Usage:
 * ```cpp
 * case "flagDependency"_h: ...
 * ```
 * @param s  Pointer to the string literal.
 * @param n  Length of the string literal.
 * @return Compile-time FNV-1a hash of the string.
 */
consteval uint64_t operator""_h(const char* s, size_t n)
{
    return fnv1a_hash(s, n);
}

/**
 * @brief Compile-time collision checker for hash dispatch tables.
 *
 * ```cpp
 * static_assert(no_hash_collisions(std::array{ "a"sv, "b"sv, "c"sv }));
 * ```
 * @tparam N  Number of keys to check.
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
 * @brief Compile-time hash dispatch table: maps string keys to values via FNV-1a.
 * @tparam T  Value type stored in each entry.
 * @tparam N  Number of entries in the dispatch table.
 */
template <typename T, std::size_t N>
struct HashDispatch
{
    /** @brief A single hash-to-value entry. */
    struct Entry
    {
        uint64_t hash; /**< Precomputed FNV-1a hash of the key. */
        T value;       /**< Value associated with the key. */
    };
    std::array<Entry, N> entries; /**< The dispatch table entries. */

    /**
     * @brief Look up a value by its string key.
     * @param key  String to hash and search for.
     * @return The matching value, or @c std::nullopt on miss.
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
 * @param destination  Raw FOMOD destination string (taken by value).
 * @return Cleaned destination with leading slashes and "./" removed.
 */
MO2_API std::string normalize_destination_for_join(std::string destination);

/**
 * @brief Resolve a \<file\>/\<folder\> node's destination, handling empty destinations
 *   and trailing-slash directory semantics, then normalize for filesystem join.
 * @param source           Source path from the FOMOD node.
 * @param raw_destination  Raw destination attribute value (may be empty).
 * @param is_file          True for \<file\> nodes, false for \<folder\> nodes.
 * @return Normalized destination path ready for filesystem join.
 */
MO2_API std::string resolve_file_destination(const std::string& source,
                                             const std::string& raw_destination,
                                             bool is_file);

/**
 * @brief Reject destination paths that would escape the mod directory via traversal
 *   or absolute paths.
 *
 * Used by both FomodService and FomodInferenceAtoms to validate file/folder
 * destinations before queuing file operations.
 * @param dest  Destination path to validate.
 * @return @c true if the destination is safe (no traversal or absolute path).
 */
MO2_API bool is_safe_destination(const std::string& dest);

/**
 * @brief Validate that @p child is strictly inside @p parent (no traversal).
 * @param parent  The directory that should contain the child.
 * @param child   The path to test.
 * @return @c true if @p child resolves to a location inside @p parent.
 * @note `weakly_canonical` errors are silently treated as `false`. This is
 *       defensive but worth knowing when debugging spurious 403/400 responses
 *       caused by transient I/O failures during canonicalization.
 */
MO2_API bool is_inside(const std::filesystem::path& parent, const std::filesystem::path& child);

/**
 * @brief Directory of the host executable (`GetModuleFileNameW(nullptr, ...)`).
 *
 * Use this for resources tied to a specific executable, such as
 * `salma.json` next to `mo2-server.exe` or the dashboard's `web/dist`
 * tree. Do **not** use it for resources that should follow the calling
 * binary inside MO2 (use module_directory() for those).
 *
 * Falls back to `std::filesystem::current_path()` if the Win32 lookup
 * fails or the platform is not Windows.
 *
 * ## Decision rule: executable vs. module directory
 *
 * - Resource follows the running EXE (config files, dashboard
 *   assets) -> `executable_directory()`.
 * - Resource follows the binary that owns the calling code
 *   (`mo2-salma.dll`'s logs, the bundled `7z.dll` it loads) ->
 *   `module_directory(&YourSymbol)`.
 *
 * Inside the MO2 plugin path the host EXE is `ModOrganizer.exe`, so
 * `executable_directory()` would point at MO2's install root rather
 * than at the salma plugin folder. That is why every salma-owned
 * resource resolves through `module_directory()`.
 *
 * @return Absolute path to the directory containing the running executable.
 */
MO2_API std::filesystem::path executable_directory();

/**
 * @brief Directory of the module containing @p anchor.
 *
 * On Windows, resolves to the directory of the DLL or EXE the address
 * lives in - regardless of the host process's working directory or the
 * host EXE's location. Use this for resources owned by the binary the
 * code itself is in (logs, a bundled `7z.dll` next to `mo2-salma.dll`).
 *
 * Falls back to `std::filesystem::current_path()` if the lookup fails
 * or the platform is not Windows.
 *
 * @param anchor Address inside the module to query (a function pointer
 *               to any symbol defined in that module is sufficient).
 * @return Absolute path to the module's containing directory.
 * @see executable_directory() for the decision rule between the two.
 */
MO2_API std::filesystem::path module_directory(const void* anchor);

/** @brief Typed error propagation alias (@c std::expected with a string error). */
template <typename T>
using Result = std::expected<T, std::string>;

}  // namespace mo2core

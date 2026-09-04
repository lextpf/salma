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
 * @brief maps enum values to exact, case-sensitive strings.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Utils
 *
 * both directions use a linear scan and return configured defaults on a miss.
 *
 * @tparam Enum enum type.
 * @tparam N entry count.
 */
template <typename Enum, std::size_t N>
struct EnumStringMap
{
    std::array<std::pair<Enum, std::string_view>, N> entries;  ///< enum and string pairs.
    Enum default_value;  ///< value returned on a string miss.
    /// string returned on an enum miss.
    std::string_view default_string = "Unknown";

    [[nodiscard]] constexpr Enum from_string(std::string_view s) const noexcept
    {
        for (const auto& [e, str] : entries)
            if (str == s)
                return e;
        return default_value;
    }

    [[nodiscard]] constexpr std::string_view to_string(Enum e) const noexcept
    {
        for (const auto& [val, str] : entries)
            if (val == e)
                return str;
        return default_string;
    }
};

/**
 * @brief provides the canonical string map for each enum type.
 * @author Alex (https://github.com/lextpf)
 *
 * a specialization must be visible at the use site. otherwise the empty primary
 * template silently returns default values.
 */
template <typename Enum>
inline constexpr auto enum_map = EnumStringMap<Enum, 0>{};

template <typename Enum>
[[nodiscard]] constexpr Enum parse_enum(std::string_view s) noexcept
{
    return enum_map<Enum>.from_string(s);
}

template <typename Enum>
[[nodiscard]] constexpr std::string_view enum_to_string(Enum e) noexcept
{
    return enum_map<Enum>.to_string(e);
}

/// exact FOMOD type map with `Optional` as the miss value.
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
 * @fn std::string to_lower(const std::string& s)
 * @brief uses bytewise C-locale conversion instead of Unicode folding.
 * @author Alex (https://github.com/lextpf)
 *
 * conversion uses the current C locale. each byte is cast to `unsigned char`
 * before `std::tolower`.
 *
 * @param s input bytes.
 * @return a lowercased copy. this is not Unicode case folding.
 */
MO2_API std::string to_lower(const std::string& s);

/**
 * @fn std::string normalize_path(const std::string& p)
 * @brief drops traversal components instead of resolving them.
 * @author Alex (https://github.com/lextpf)
 *
 * the result is lowercase, uses forward slashes, and has no leading, trailing,
 * repeated, dot, or dot-dot components. dot-dot components are removed rather
 * than resolved, so `a/b/../c` becomes `a/b/c`.
 *
 * ### :material-transit-connection-variant: normalization flow
 *
 * ```mermaid
 * flowchart LR
 *     raw --> lower[lowercase]
 *     lower --> slash[use forward slashes]
 *     slash --> trim[trim edge separators]
 *     trim --> collapse[collapse repeats]
 *     collapse --> segments[remove dot components]
 * ```
 *
 * @param p archive or FOMOD path.
 * @return a comparison key, or empty when no ordinary component remains.
 */
MO2_API std::string normalize_path(const std::string& p);

/**
 * @fn std::string random_hex_string(size_t length)
 * @brief uses per-thread MT19937 output that is unsuitable for secrets.
 * @author Alex (https://github.com/lextpf)
 *
 * each thread owns an MT19937 instance seeded on first use.
 *
 * @param length output length in characters. zero returns an empty string.
 * @return exactly the requested number of hexadecimal characters.
 * @warning this is not a cryptographic generator. do not use it for secrets.
 */
MO2_API std::string random_hex_string(size_t length = 12);

MO2_API PluginType parse_plugin_type_string(const std::string& type_name);

MO2_API std::string_view plugin_type_to_string(PluginType type);

/**
 * @fn constexpr uint64_t fnv1a_hash(const char* data, size_t size)
 * @brief uses unsigned bytes and modulo-64-bit overflow.
 * @author Alex (https://github.com/lextpf)
 *
 * bytes are unsigned and multiplication wraps modulo 2^64.
 *
 * @param data input bytes. non-null when `size` is nonzero.
 * @param size input length in bytes.
 * @return the hash, or the offset basis for an empty input.
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

consteval uint64_t operator""_h(const char* s, size_t n)
{
    return fnv1a_hash(s, n);
}

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
 * @brief maps string hashes to values at compile time.
 * @author Alex (https://github.com/lextpf)
 * @ingroup Utils
 *
 * entries do not retain keys. verify the key set with `no_hash_collisions`.
 *
 * @tparam T stored value type.
 * @tparam N entry count.
 */
template <typename T, std::size_t N>
struct HashDispatch
{
    struct Entry
    {
        uint64_t hash;  ///< precomputed FNV-1a hash.
        T value;        ///< associated value.
    };
    std::array<Entry, N> entries;

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
 * @fn std::string normalize_destination_for_join(std::string destination)
 * @brief reanchors leading separators beneath the mod root.
 * @author Alex (https://github.com/lextpf)
 *
 * leading separators and dot-separator prefixes are removed. case is retained.
 *
 * @param destination raw destination.
 * @return a relative destination. empty means the mod root.
 */
MO2_API std::string normalize_destination_for_join(std::string destination);

/**
 * @fn std::string resolve_file_destination(const std::string&, const std::string&, bool)
 * @brief preserves a source filename when a file destination omits it.
 * @author Alex (https://github.com/lextpf)
 *
 * for files, an empty destination keeps the source filename and a trailing
 * separator appends it. folder destinations pass through unchanged before
 * normalization.
 *
 * @param source source node path.
 * @param raw_destination destination attribute, or empty.
 * @param is_file `true` for a file node.
 * @return destination ready to join under the mod root.
 */
MO2_API std::string resolve_file_destination(const std::string& source,
                                             const std::string& raw_destination,
                                             bool is_file);

/**
 * @fn bool is_safe_destination(const std::string& dest)
 * @brief allows reanchored POSIX paths but rejects Windows drive paths.
 * @author Alex (https://github.com/lextpf)
 *
 * normalization removes traversal components and re-anchors leading separators.
 * drive-qualified Windows paths remain unsafe. an empty result means the mod root.
 *
 * @param dest destination to inspect.
 * @return `false` only when a drive-qualified or retained absolute path remains.
 */
MO2_API bool is_safe_destination(const std::string& dest);

/**
 * @fn bool is_safe_mod_name(const std::string& name)
 * @brief rejects names that Windows aliases or trims.
 * @author Alex (https://github.com/lextpf)
 *
 * rejects empty names, edge whitespace, separators, absolute paths, dot names,
 * trailing dots, and Windows device names. the device-name check ignores case
 * and the final extension.
 *
 * @param name candidate name.
 * @return `true` for a safe single component.
 */
MO2_API bool is_safe_mod_name(const std::string& name);

/**
 * @fn bool is_inside(const std::filesystem::path&, const std::filesystem::path&)
 * @brief fails closed on canonicalization errors and dot-prefixed siblings.
 * @author Alex (https://github.com/lextpf)
 *
 * equal paths pass. a first relative component that starts with two dots fails,
 * including a valid name such as `..foo`. canonicalization errors fail closed.
 *
 * @param parent expected container.
 * @param child path to inspect.
 * @return `true` when the canonical child is equal to or inside the parent.
 */
MO2_API bool is_inside(const std::filesystem::path& parent, const std::filesystem::path& child);

/**
 * @fn std::filesystem::path executable_directory()
 * @brief falls back to the working directory when platform lookup fails.
 * @author Alex (https://github.com/lextpf)
 *
 * use this for resources owned by the running executable. lookup failure and
 * non-Windows builds use the process working directory.
 *
 * @return the resolved directory. the value is not cached.
 */
MO2_API std::filesystem::path executable_directory();

/**
 * @fn std::filesystem::path module_directory(const void* anchor)
 * @brief falls back to the working directory when module lookup fails.
 * @author Alex (https://github.com/lextpf)
 *
 * use this for resources owned by a DLL or executable. the lookup does not retain
 * the module. failure and non-Windows builds use the process working directory.
 *
 * @param anchor address inside the target module.
 * @return the resolved directory. the value is not cached.
 * @see executable_directory
 */
MO2_API std::filesystem::path module_directory(const void* anchor);

template <typename T>
using Result = std::expected<T, std::string>;

}  // namespace mo2core

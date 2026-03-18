#pragma once

#include "Export.h"
#include "Types.h"

#include <array>
#include <expected>
#include <optional>
#include <pugixml.hpp>
#include <string>
#include <string_view>
#include <vector>

namespace mo2core
{

/// Bidirectional constexpr enum <-> string mapping.
template <typename Enum, std::size_t N>
struct EnumStringMap
{
    std::array<std::pair<Enum, std::string_view>, N> entries;
    Enum default_value;
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

/// Lowercase a string (safe for MSVC plain char via unsigned char cast).
MO2_API std::string to_lower(const std::string& s);

/// Normalize an archive/mod path: lowercase, backslash-to-forward-slash,
/// strip leading "./" and "/", strip trailing "/", collapse "//".
MO2_API std::string normalize_path(const std::string& p);

/// Generate a random hex string of the given length using a thread-local RNG.
MO2_API std::string random_hex_string(size_t length = 12);

/// Respect the FOMOD `order` attribute on installSteps, optionalFileGroups, and plugins.
/// Values: "Ascending" (default, alphabetical by name), "Descending", "Explicit" (document order).
MO2_API std::vector<pugi::xml_node> get_ordered_nodes(const pugi::xml_node& parent,
                                                      const char* child_name);

/// Parse an XML boolean attribute using XML Schema semantics.
/// Returns true for "true"/"1" (case-insensitive), false otherwise.
MO2_API bool xml_bool_attribute_true(const pugi::xml_attribute& attr);

/// Map a FOMOD plugin type name string to its PluginType enum value.
MO2_API PluginType parse_plugin_type_string(const std::string& type_name);

/// Map a PluginType enum value to its FOMOD type name string.
MO2_API std::string_view plugin_type_to_string(PluginType type);

/// FNV-1a 64-bit hash.
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

/// Compile-time string hash literal for switch-case dispatch.
/// Usage: case "flagDependency"_h: ...
consteval uint64_t operator""_h(const char* s, size_t n)
{
    return fnv1a_hash(s, n);
}

/// Compile-time collision checker for hash dispatch tables.
/// static_assert(no_hash_collisions(std::array{ "a"sv, "b"sv, "c"sv }));
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

/// Compile-time hash dispatch table: maps string keys to values via FNV-1a.
template <typename T, std::size_t N>
struct HashDispatch
{
    struct Entry
    {
        uint64_t hash;
        T value;
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

/// Typed error propagation alias.
template <typename T>
using Result = std::expected<T, std::string>;

}  // namespace mo2core

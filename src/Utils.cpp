#include "Utils.h"

#include <algorithm>
#include <random>
#include <ranges>

namespace mo2core
{

std::string to_lower(const std::string& s)
{
    std::string out = s;
    // Unsigned char cast is required - std::tolower is undefined for negative values
    // from plain char on MSVC.
    std::transform(
        out.begin(), out.end(), out.begin(), [](unsigned char c) { return std::tolower(c); });
    return out;
}

std::string normalize_path(const std::string& p)
{
    std::string out = to_lower(p);
    std::replace(out.begin(), out.end(), '\\', '/');
    // Strip leading "./" or "/" prefixes that some archivers emit
    while (out.starts_with("./"))
        out = out.substr(2);
    while (out.starts_with("/"))
        out = out.substr(1);
    // Strip trailing "/"
    while (out.ends_with("/"))
        out.pop_back();
    // Single-pass collapse of consecutive slashes
    {
        std::string collapsed;
        collapsed.reserve(out.size());
        for (char c : out)
        {
            if (c == '/' && !collapsed.empty() && collapsed.back() == '/')
                continue;
            collapsed.push_back(c);
        }
        out = std::move(collapsed);
    }

    // Remove "." and ".." path components to prevent directory traversal
    {
        std::vector<std::string> parts;
        size_t start = 0;
        while (start < out.size())
        {
            auto end = out.find('/', start);
            if (end == std::string::npos)
                end = out.size();
            auto seg = out.substr(start, end - start);
            if (!seg.empty() && seg != "." && seg != "..")
                parts.push_back(std::move(seg));
            start = end + 1;
        }
        out.clear();
        for (size_t i = 0; i < parts.size(); ++i)
        {
            if (i > 0)
                out += '/';
            out += parts[i];
        }
    }

    return out;
}

std::string random_hex_string(size_t length)
{
    static const char hex[] = "0123456789abcdef";
    // thread_local avoids contention when multiple threads extract concurrently
    thread_local std::mt19937 rng{std::random_device{}()};
    std::uniform_int_distribution<int> dist(0, 15);

    std::string out;
    out.reserve(length);
    for (size_t i = 0; i < length; ++i)
    {
        out.push_back(hex[static_cast<size_t>(dist(rng))]);
    }
    return out;
}

std::vector<pugi::xml_node> get_ordered_nodes(const pugi::xml_node& parent, const char* child_name)
{
    std::string order = parent.attribute("order").as_string("Ascending");
    std::vector<pugi::xml_node> nodes;
    for (auto child : parent.children(child_name))
    {
        nodes.push_back(child);
    }

    auto name_projection = [](const pugi::xml_node& n)
    { return std::string(n.attribute("name").as_string()); };

    if (order == "Descending")
        std::ranges::sort(nodes, std::ranges::greater{}, name_projection);
    else if (order == "Ascending")
        std::ranges::sort(nodes, std::ranges::less{}, name_projection);
    // "Explicit" = document order (already correct from iteration)
    return nodes;
}

bool xml_bool_attribute_true(const pugi::xml_attribute& attr)
{
    if (!attr)
        return false;

    auto value = to_lower(attr.as_string());
    return value == "true" || value == "1";
}

PluginType parse_plugin_type_string(const std::string& type_name)
{
    return parse_enum<PluginType>(type_name);
}

std::string_view plugin_type_to_string(PluginType type)
{
    return enum_to_string(type);
}

std::string normalize_destination_for_join(std::string destination)
{
    // FOMOD destinations are mod-root-relative. Values like "\" or "/"
    // mean "root", not an absolute filesystem path.
    while (!destination.empty() && (destination.front() == '\\' || destination.front() == '/'))
    {
        destination.erase(destination.begin());
    }
    while (destination.starts_with("./") || destination.starts_with(".\\"))
    {
        destination = destination.substr(2);
    }
    return destination;
}

std::string resolve_file_destination(const std::string& source,
                                     const std::string& raw_destination,
                                     bool is_file)
{
    std::string destination = raw_destination;
    if (is_file && destination.empty())
    {
        auto slash = source.find_last_of("/\\");
        destination = (slash != std::string::npos) ? source.substr(slash + 1) : source;
    }
    else if (is_file && !destination.empty() &&
             (destination.back() == '/' || destination.back() == '\\'))
    {
        auto slash = source.find_last_of("/\\");
        auto filename = (slash != std::string::npos) ? source.substr(slash + 1) : source;
        destination += filename;
    }
    return normalize_destination_for_join(destination);
}

bool is_safe_destination(const std::string& dest)
{
    if (dest.empty())
        return true;
    auto norm = normalize_path(dest);
    // After normalize_path, ".." segments survive only if they would escape root.
    if (norm.starts_with("..") || norm.find("/../") != std::string::npos || norm.ends_with("/.."))
        return false;
    // Absolute paths (e.g., "/etc/passwd" or "C:/...")
    if (norm.front() == '/' || (norm.size() >= 2 && norm[1] == ':'))
        return false;
    return true;
}

bool is_inside(const std::filesystem::path& parent, const std::filesystem::path& child)
{
    std::error_code ec;
    auto canonical_child = std::filesystem::weakly_canonical(child, ec);
    if (ec)
        return false;
    auto canonical_parent = std::filesystem::weakly_canonical(parent, ec);
    if (ec)
        return false;
    auto rel = canonical_child.lexically_relative(canonical_parent);
    return !rel.empty() && !rel.string().starts_with("..");
}

}  // namespace mo2core

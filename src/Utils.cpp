#include "Utils.h"

#include <algorithm>
#include <random>

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
    // Collapse consecutive slashes (e.g. from dest="textures\lod\" + "/" + rel)
    size_t pos = 0;
    while ((pos = out.find("//", pos)) != std::string::npos)
    {
        out.erase(pos, 1);
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

    if (order == "Descending")
    {
        std::sort(nodes.begin(),
                  nodes.end(),
                  [](const pugi::xml_node& a, const pugi::xml_node& b)
                  {
                      return std::string(a.attribute("name").as_string()) >
                             std::string(b.attribute("name").as_string());
                  });
    }
    else if (order == "Ascending")
    {
        std::sort(nodes.begin(),
                  nodes.end(),
                  [](const pugi::xml_node& a, const pugi::xml_node& b)
                  {
                      return std::string(a.attribute("name").as_string()) <
                             std::string(b.attribute("name").as_string());
                  });
    }
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
    if (type_name == "Required")
        return PluginType::Required;
    if (type_name == "Recommended")
        return PluginType::Recommended;
    if (type_name == "Optional")
        return PluginType::Optional;
    if (type_name == "NotUsable")
        return PluginType::NotUsable;
    if (type_name == "CouldBeUsable")
        return PluginType::CouldBeUsable;
    return PluginType::Optional;
}

uint64_t fnv1a_hash(const char* data, size_t size)
{
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < size; ++i)
    {
        hash ^= static_cast<uint64_t>(static_cast<unsigned char>(data[i]));
        hash *= 1099511628211ULL;
    }
    return hash;
}

}  // namespace mo2core

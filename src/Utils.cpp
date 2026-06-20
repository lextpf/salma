#include "Utils.hpp"

#include <algorithm>
#include <cctype>
#include <random>
#include <ranges>
#include <unordered_set>

#ifdef _WIN32
#include <windows.h>
#endif

namespace mo2core
{

#ifdef _WIN32
namespace
{

// double the module-path buffer at most five times. return empty on truncation.
std::filesystem::path module_path_for(HMODULE hMod)
{
    std::wstring buf(MAX_PATH, L'\0');
    DWORD len = GetModuleFileNameW(hMod, buf.data(), static_cast<DWORD>(buf.size()));
    constexpr int kMaxRetries = 5;
    int retries = 0;
    while (len >= buf.size() && retries < kMaxRetries)
    {
        buf.resize(buf.size() * 2);
        len = GetModuleFileNameW(hMod, buf.data(), static_cast<DWORD>(buf.size()));
        ++retries;
    }
    if (len > 0 && retries < kMaxRetries)
    {
        buf.resize(len);
        return std::filesystem::path(buf).parent_path();
    }
    return {};
}

}  // namespace
#endif

std::string to_lower(const std::string& s)
{
    std::string out = s;
    // std::tolower is undefined for a negative plain char.
    std::transform(
        out.begin(), out.end(), out.begin(), [](unsigned char c) { return std::tolower(c); });
    return out;
}

std::string normalize_path(const std::string& p)
{
    std::string out = to_lower(p);
    std::replace(out.begin(), out.end(), '\\', '/');
    // strip prefixes emitted by some archivers.
    while (out.starts_with("./"))
        out = out.substr(2);
    while (out.starts_with("/"))
        out = out.substr(1);
    // strip the trailing separator.
    while (out.ends_with("/"))
        out.pop_back();
    // collapse consecutive separators in one pass.
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

    // remove traversal components.
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
    // keep one generator per worker to avoid serialization. MT19937 is not
    // cryptographically secure; do not use this output as a sampled secret.
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
    // FOMOD treats a separator-only destination as the mod root.
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
    // normalization removes traversal and reanchors a leading POSIX separator.
    // reject drive-qualified Windows paths; retain the slash guard if normalization changes.
    if (norm.empty())
        return true;
    if (norm.front() == '/' || (norm.size() >= 2 && norm[1] == ':'))
        return false;
    return true;
}

bool is_safe_mod_name(const std::string& name)
{
    if (name.empty())
        return false;

    // reject whitespace that Windows can trim during file creation.
    auto is_ws = [](unsigned char c) { return std::isspace(c) != 0; };
    if (is_ws(static_cast<unsigned char>(name.front())) ||
        is_ws(static_cast<unsigned char>(name.back())))
    {
        return false;
    }

    // reject separators and absolute paths.
    if (name.find('/') != std::string::npos || name.find('\\') != std::string::npos)
        return false;
    if (std::filesystem::path(name).is_absolute())
        return false;

    // reject dot directory names.
    if (name == "." || name == "..")
        return false;

    // reject a trailing dot that Windows can strip.
    if (name.back() == '.')
        return false;

    // compare the lowercase stem so names such as CON.txt remain reserved.
    // keep this list synchronized with RESERVED_NAMES in installation_service.rs.
    static const std::unordered_set<std::string> kReservedNames = {
        "con",  "prn",  "aux",  "nul",  "com1", "com2", "com3", "com4", "com5", "com6", "com7",
        "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    };
    auto stem = to_lower(name);
    if (auto dot = stem.rfind('.'); dot != std::string::npos)
        stem = stem.substr(0, dot);
    if (kReservedNames.contains(stem))
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

std::filesystem::path executable_directory()
{
#ifdef _WIN32
    auto dir = module_path_for(nullptr);
    if (!dir.empty())
        return dir;
#endif
    return std::filesystem::current_path();
}

std::filesystem::path module_directory(const void* anchor)
{
#ifdef _WIN32
    HMODULE hMod = nullptr;
    if (GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(anchor),
            &hMod))
    {
        auto dir = module_path_for(hMod);
        if (!dir.empty())
            return dir;
    }
#else
    (void)anchor;
#endif
    return std::filesystem::current_path();
}

}  // namespace mo2core
